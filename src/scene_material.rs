//! `ufbx_material`, `ufbx_texture`, `ufbx_video` and `ufbx_texture_file`
//! records of the engine-converted profile, ported operation for operation from
//! pinned ufbx 0.23.1 (`ufbx.c`, `ufbx_real` = `double`, reference build
//! `-ffp-contract=off`).
//!
//! Load order reproduced: `ufbxi_read_material`, `ufbxi_read_texture`,
//! `ufbxi_read_layered_texture` and `ufbxi_read_video` (with
//! `ufbxi_read_embedded_blob`); the material and texture part of
//! `ufbxi_finalize_scene` (shader type from the shading model and 3ds Max
//! class IDs, `ufbxi_fetch_textures`, `ufbxi_resolve_file_content`,
//! `ufbxi_resolve_filenames`, texture layers, `ufbxi_insert_texture_file`,
//! `ufbxi_pop_texture_files`, `ufbxi_sort_material_textures`,
//! `ufbxi_fetch_maps`, `ufbxi_fetch_file_textures`); and `ufbxi_update_texture`
//! from `ufbxi_update_scene`.
//!
//! Not ported (no corpus input reaches them; each is reported on stderr and
//! leaves the affected record as far as computed): shader textures
//! (OSL/select-output branches of `ufbxi_finalize_shader_texture`), ShaderFX
//! mapping tables and Blender-specific Phong mapping. Legacy
//! `LayerElement*Texture` bindings are patched into the material projection
//! inputs by `legacy::material_textures`.
use crate::canon::{s, Obj, V};
use crate::canon_dom::{node_content, node_data, node_string, NodeData};
use crate::canon_scene::ElementType;
use crate::scene_node::{self as sn, Transform};
use crate::scene_property::{AuthoredProperty, PropertyLayers, Type};
use crate::syntax::Document;
use crate::{api_alloc::Budget, Error};

const NO_INDEX: u64 = u32::MAX as u64;
/// `ufbx_load_opts.path_separator` default on the reference platform (Linux).
const PATH_SEPARATOR: u8 = b'/';
/// `UFBX_BLEND_OVERLAY`.
const BLEND_OVERLAY: i64 = 30;

const PROP_FLAG_VALUE_REAL: u32 = 0x10_0000;
const PROP_FLAG_VALUE_VEC2: u32 = 0x20_0000;
const PROP_FLAG_VALUE_VEC3: u32 = 0x40_0000;
const PROP_FLAG_VALUE_VEC4: u32 = 0x80_0000;

/// `ufbx_material_fbx_maps` field order (`UFBX_MATERIAL_FBX_*`).
pub(crate) const FBX_MAPS: [&str; 20] = [
    "diffuse_factor",
    "diffuse_color",
    "specular_factor",
    "specular_color",
    "specular_exponent",
    "reflection_factor",
    "reflection_color",
    "transparency_factor",
    "transparency_color",
    "emission_factor",
    "emission_color",
    "ambient_factor",
    "ambient_color",
    "normal_map",
    "bump",
    "bump_factor",
    "displacement_factor",
    "displacement",
    "vector_displacement_factor",
    "vector_displacement",
];
/// `ufbx_material_pbr_maps` field order (`UFBX_MATERIAL_PBR_*`).
pub(crate) const PBR_MAPS: [&str; 56] = [
    "base_factor",
    "base_color",
    "roughness",
    "metalness",
    "diffuse_roughness",
    "specular_factor",
    "specular_color",
    "specular_ior",
    "specular_anisotropy",
    "specular_rotation",
    "transmission_factor",
    "transmission_color",
    "transmission_depth",
    "transmission_scatter",
    "transmission_scatter_anisotropy",
    "transmission_dispersion",
    "transmission_roughness",
    "transmission_extra_roughness",
    "transmission_priority",
    "transmission_enable_in_aov",
    "subsurface_factor",
    "subsurface_color",
    "subsurface_radius",
    "subsurface_scale",
    "subsurface_anisotropy",
    "subsurface_tint_color",
    "subsurface_type",
    "sheen_factor",
    "sheen_color",
    "sheen_roughness",
    "coat_factor",
    "coat_color",
    "coat_roughness",
    "coat_ior",
    "coat_anisotropy",
    "coat_rotation",
    "coat_normal",
    "coat_affect_base_color",
    "coat_affect_base_roughness",
    "thin_film_factor",
    "thin_film_thickness",
    "thin_film_ior",
    "emission_factor",
    "emission_color",
    "opacity",
    "indirect_diffuse",
    "indirect_specular",
    "normal_map",
    "tangent_map",
    "displacement_map",
    "matte_factor",
    "matte_color",
    "ambient_occlusion",
    "glossiness",
    "coat_glossiness",
    "transmission_glossiness",
];
/// `ufbx_material_features` field order (`UFBX_MATERIAL_FEATURE_*`).
pub(crate) const FEATURES: [&str; 23] = [
    "pbr",
    "metalness",
    "diffuse",
    "specular",
    "emission",
    "transmission",
    "coat",
    "sheen",
    "opacity",
    "ambient_occlusion",
    "matte",
    "unlit",
    "ior",
    "diffuse_roughness",
    "transmission_roughness",
    "thin_walled",
    "caustics",
    "exit_to_background",
    "internal_reflections",
    "double_sided",
    "roughness_as_glossiness",
    "coat_roughness_as_glossiness",
    "transmission_roughness_as_glossiness",
];

fn fbx(name: &str) -> usize {
    FBX_MAPS.iter().position(|&n| n == name).unwrap_or(0)
}
fn pbr(name: &str) -> usize {
    PBR_MAPS.iter().position(|&n| n == name).unwrap_or(0)
}
fn feature(name: &str) -> usize {
    FEATURES.iter().position(|&n| n == name).unwrap_or(0)
}

/// `ufbxi_shader_mapping_flag`.
const DEFAULT_W_1: u8 = 0x1;
const WIDEN_TO_RGB: u8 = 0x2;
const MULTIPLY_VALUE: u8 = 0x4;
const FEATURE_INVERTED: u8 = 0x1;
const FEATURE_IF_AROUND_1: u8 = 0x8;
/// `ufbxi_mat_transform`.
#[derive(Clone, Copy, PartialEq)]
enum MatTransform {
    None,
    UnknownShininess,
}
/// One `ufbxi_shader_mapping`: target map name, flags, transform, property.
type Mapping = (&'static str, u8, MatTransform, &'static str);
const N: MatTransform = MatTransform::None;

/// `ufbxi_base_fbx_mapping`.
const BASE_FBX_MAPPING: &[Mapping] = &[
    ("diffuse_color", DEFAULT_W_1, N, "Diffuse"),
    ("diffuse_color", DEFAULT_W_1, N, "DiffuseColor"),
    ("diffuse_factor", 0, N, "DiffuseFactor"),
    ("specular_color", DEFAULT_W_1, N, "Specular"),
    ("specular_color", DEFAULT_W_1, N, "SpecularColor"),
    ("specular_factor", 0, N, "SpecularFactor"),
    ("specular_exponent", 0, N, "Shininess"),
    ("specular_exponent", 0, N, "ShininessExponent"),
    ("reflection_color", DEFAULT_W_1, N, "Reflection"),
    ("reflection_color", DEFAULT_W_1, N, "ReflectionColor"),
    ("reflection_factor", 0, N, "ReflectionFactor"),
    ("transparency_color", DEFAULT_W_1, N, "Transparent"),
    ("transparency_color", DEFAULT_W_1, N, "TransparentColor"),
    ("transparency_factor", 0, N, "TransparentFactor"),
    ("transparency_factor", 0, N, "TransparencyFactor"),
    ("emission_color", DEFAULT_W_1, N, "Emissive"),
    ("emission_color", DEFAULT_W_1, N, "EmissiveColor"),
    ("emission_factor", 0, N, "EmissiveFactor"),
    ("ambient_color", DEFAULT_W_1, N, "Ambient"),
    ("ambient_color", DEFAULT_W_1, N, "AmbientColor"),
    ("ambient_factor", 0, N, "AmbientFactor"),
    ("normal_map", 0, N, "NormalMap"),
    ("bump", 0, N, "Bump"),
    ("bump_factor", 0, N, "BumpFactor"),
    ("displacement", 0, N, "Displacement"),
    ("displacement_factor", 0, N, "DisplacementFactor"),
    ("vector_displacement", 0, N, "VectorDisplacement"),
    (
        "vector_displacement_factor",
        0,
        N,
        "VectorDisplacementFactor",
    ),
];
/// `ufbxi_fbx_lambert_shader_pbr_mapping`.
const LAMBERT_PBR_MAPPING: &[Mapping] = &[
    ("base_color", DEFAULT_W_1, N, "Diffuse"),
    ("base_color", DEFAULT_W_1, N, "DiffuseColor"),
    ("base_factor", 0, N, "DiffuseFactor"),
    ("transmission_color", DEFAULT_W_1, N, "Transparent"),
    ("transmission_color", DEFAULT_W_1, N, "TransparentColor"),
    ("transmission_factor", 0, N, "TransparentFactor"),
    ("transmission_factor", 0, N, "TransparencyFactor"),
    ("emission_color", DEFAULT_W_1, N, "Emissive"),
    ("emission_color", DEFAULT_W_1, N, "EmissiveColor"),
    ("emission_factor", 0, N, "EmissiveFactor"),
    ("normal_map", 0, N, "NormalMap"),
];
/// `ufbxi_fbx_phong_shader_pbr_mapping`.
const PHONG_PBR_MAPPING: &[Mapping] = &[
    ("base_color", DEFAULT_W_1, N, "Diffuse"),
    ("base_color", DEFAULT_W_1, N, "DiffuseColor"),
    ("base_factor", 0, N, "DiffuseFactor"),
    ("specular_color", DEFAULT_W_1, N, "Specular"),
    ("specular_color", DEFAULT_W_1, N, "SpecularColor"),
    ("specular_factor", 0, N, "SpecularFactor"),
    ("roughness", 0, MatTransform::UnknownShininess, "Shininess"),
    (
        "roughness",
        0,
        MatTransform::UnknownShininess,
        "ShininessExponent",
    ),
    ("transmission_color", DEFAULT_W_1, N, "Transparent"),
    ("transmission_color", DEFAULT_W_1, N, "TransparentColor"),
    ("transmission_factor", 0, N, "TransparentFactor"),
    ("transmission_factor", 0, N, "TransparencyFactor"),
    ("emission_color", DEFAULT_W_1, N, "Emissive"),
    ("emission_color", DEFAULT_W_1, N, "EmissiveColor"),
    ("emission_factor", 0, N, "EmissiveFactor"),
    ("normal_map", 0, N, "NormalMap"),
];

/// `ufbxi_3ds_max_physical_material_pbr_mapping` (ufbx 0.23.1).
const PHYSICAL_PBR_MAPPING: &[Mapping] = &[
    ("base_factor", 0, N, "base_weight"),
    ("base_color", DEFAULT_W_1, N, "base_color"),
    ("roughness", 0, N, "roughness"),
    ("diffuse_roughness", 0, N, "diff_rough"),
    ("diffuse_roughness", 0, N, "diff_roughness"),
    ("metalness", 0, N, "metalness"),
    ("specular_factor", 0, N, "reflectivity"),
    ("specular_color", DEFAULT_W_1, N, "refl_color"),
    ("specular_anisotropy", 0, N, "anisotropy"),
    ("specular_rotation", 0, N, "aniso_angle"),
    ("specular_rotation", 0, N, "anisoangle"),
    ("specular_ior", 0, N, "trans_ior"),
    ("transmission_factor", 0, N, "transparency"),
    ("transmission_color", DEFAULT_W_1, N, "trans_color"),
    ("transmission_depth", 0, N, "trans_depth"),
    ("transmission_roughness", 0, N, "trans_rough"),
    ("transmission_roughness", 0, N, "trans_roughness"),
    ("subsurface_factor", 0, N, "scattering"),
    ("subsurface_tint_color", DEFAULT_W_1, N, "sss_color"),
    ("subsurface_color", DEFAULT_W_1, N, "sss_scatter_color"),
    ("subsurface_radius", WIDEN_TO_RGB, N, "sss_depth"),
    ("subsurface_scale", 0, N, "sss_scale"),
    ("coat_factor", 0, N, "coat"),
    ("coat_factor", 0, N, "coating"),
    ("coat_color", DEFAULT_W_1, N, "coat_color"),
    ("coat_roughness", 0, N, "coat_rough"),
    ("coat_roughness", 0, N, "coat_roughness"),
    ("coat_ior", 0, N, "coat_ior"),
    ("coat_normal", 0, N, "coat_bump"),
    ("coat_normal", 0, N, "clearcoat_bump_map_amt"),
    (
        "coat_affect_base_color",
        DEFAULT_W_1,
        N,
        "coat_affect_color",
    ),
    ("coat_affect_base_roughness", 0, N, "coat_affect_roughness"),
    ("emission_factor", 0, N, "emission"),
    ("emission_color", DEFAULT_W_1, N, "emit_color"),
    ("opacity", WIDEN_TO_RGB, N, "cutout"),
    ("normal_map", 0, N, "bump"),
    ("normal_map", 0, N, "bump_map_amt"),
    ("displacement_map", 0, N, "displacement"),
    ("displacement_map", 0, N, "displacement_map_amt"),
    ("subsurface_type", 0, N, "subsurfaceType"),
];

/// `ufbxi_3ds_max_physical_material_features` (ufbx 0.23.1).
const PHYSICAL_FEATURES: &[Mapping] = &[
    ("thin_walled", 0, N, "thin_walled"),
    ("specular", 0, N, "material_mode"),
    ("diffuse_roughness", 0, N, "material_mode"),
    (
        "transmission_roughness",
        FEATURE_INVERTED,
        N,
        "trans_roughness_lock",
    ),
    ("roughness_as_glossiness", 0, N, "roughness_inv"),
    (
        "transmission_roughness_as_glossiness",
        0,
        N,
        "trans_roughness_inv",
    ),
    ("coat_roughness_as_glossiness", 0, N, "coat_roughness_inv"),
];

/// `ufbxi_3ds_max_pbr_metal_rough_pbr_mapping` (ufbx 0.23.1).
const METAL_ROUGH_PBR_MAPPING: &[Mapping] = &[
    ("base_color", DEFAULT_W_1, N, "base_color"),
    ("base_color", DEFAULT_W_1, N, "baseColor"),
    ("roughness", 0, N, "roughness"),
    ("roughness", 0, N, "Roughness_Map"),
    ("metalness", 0, N, "metalness"),
    ("ambient_occlusion", 0, N, "ao"),
    ("normal_map", 0, N, "norm"),
    ("emission_color", DEFAULT_W_1, N, "emit_color"),
    ("displacement_map", 0, N, "displacement"),
    ("displacement_map", 0, N, "displacement_amt"),
    ("opacity", WIDEN_TO_RGB, N, "opacity"),
];

/// `ufbxi_3ds_max_pbr_spec_gloss_pbr_mapping` (ufbx 0.23.1).
const SPEC_GLOSS_PBR_MAPPING: &[Mapping] = &[
    ("base_color", DEFAULT_W_1, N, "base_color"),
    ("base_color", DEFAULT_W_1, N, "baseColor"),
    ("specular_color", DEFAULT_W_1, N, "Specular"),
    ("specular_color", DEFAULT_W_1, N, "specular"),
    ("roughness", 0, N, "glossiness"),
    ("ambient_occlusion", 0, N, "ao"),
    ("normal_map", 0, N, "norm"),
    ("emission_color", DEFAULT_W_1, N, "emit_color"),
    ("displacement_map", 0, N, "displacement"),
    ("displacement_map", 0, N, "displacement_amt"),
    ("opacity", WIDEN_TO_RGB, N, "opacity"),
];

/// `ufbxi_3ds_max_pbr_features` (ufbx 0.23.1).
const MAX_PBR_FEATURES: &[Mapping] = &[(
    "roughness_as_glossiness",
    FEATURE_IF_AROUND_1,
    N,
    "useGlossiness",
)];

/// `ufbxi_gltf_material_pbr_mapping` (ufbx 0.23.1).
const GLTF_PBR_MAPPING: &[Mapping] = &[
    ("base_color", DEFAULT_W_1, N, "main|baseColor"),
    ("roughness", 0, N, "main|roughness"),
    ("metalness", 0, N, "main|metalness"),
    ("normal_map", 0, N, "main|normal"),
    ("ambient_occlusion", 0, N, "main|ambientOcclusion"),
    ("emission_color", DEFAULT_W_1, N, "main|emission"),
    ("emission_color", DEFAULT_W_1, N, "main|emissionColor"),
    ("opacity", WIDEN_TO_RGB, N, "main|Alpha"),
    ("coat_factor", 0, N, "extension|clearcoat"),
    ("coat_roughness", 0, N, "extension|clearcoatRoughness"),
    ("coat_normal", 0, N, "extension|clearcoatNormal"),
    ("sheen_color", DEFAULT_W_1, N, "extension|sheenColor"),
    ("sheen_roughness", 0, N, "extension|sheenRoughness"),
    ("specular_factor", 0, N, "extension|specular"),
    ("specular_factor", 0, N, "extension|Specular"),
    ("specular_color", DEFAULT_W_1, N, "extension|specularcolor"),
    ("specular_color", DEFAULT_W_1, N, "extension|specularColor"),
    ("transmission_factor", 0, N, "extension|transmission"),
    ("specular_ior", 0, N, "extension|indexOfRefraction"),
];

/// `ufbxi_gltf_material_features` (ufbx 0.23.1).
const GLTF_FEATURES: &[Mapping] = &[
    ("double_sided", 0, N, "main|DoubleSided"),
    ("sheen", 0, N, "extension|enableSheen"),
    ("coat", 0, N, "extension|enableClearCoat"),
    ("transmission", 0, N, "extension|enableTransmission"),
    ("ior", 0, N, "extension|enableIndexOfRefraction"),
    ("specular", 0, N, "extension|enableSpecular"),
    ("unlit", 0, N, "extension|unlit"),
];

/// `ufbxi_openpbr_material_pbr_mapping` (ufbx 0.23.1).
const OPENPBR_MAPPING: &[Mapping] = &[
    ("base_factor", 0, N, "base_weight"),
    ("base_color", DEFAULT_W_1, N, "base_color"),
    ("roughness", 0, N, "specular_roughness"),
    ("diffuse_roughness", 0, N, "base_diffuse_roughness"),
    ("metalness", 0, N, "base_metalness"),
    ("specular_factor", 0, N, "specular_weight"),
    ("specular_color", DEFAULT_W_1, N, "specular_color"),
    ("specular_anisotropy", 0, N, "specular_roughness_anisotropy"),
    ("specular_ior", 0, N, "specular_ior"),
    ("transmission_factor", 0, N, "transmission_weight"),
    ("transmission_color", DEFAULT_W_1, N, "transmission_color"),
    ("transmission_depth", 0, N, "transmission_depth"),
    (
        "transmission_scatter",
        WIDEN_TO_RGB,
        N,
        "transmission_scatter",
    ),
    (
        "transmission_scatter_anisotropy",
        0,
        N,
        "transmission_scatter_anisotropy",
    ),
    (
        "transmission_dispersion",
        0,
        N,
        "transmission_dispersion_scale",
    ),
    ("subsurface_factor", 0, N, "subsurface_weight"),
    ("subsurface_color", DEFAULT_W_1, N, "subsurface_color"),
    (
        "subsurface_radius",
        WIDEN_TO_RGB,
        N,
        "subsurface_radius_scale",
    ),
    ("subsurface_scale", 0, N, "subsurface_radius"),
    (
        "subsurface_anisotropy",
        0,
        N,
        "subsurface_scatter_anisotropy",
    ),
    ("coat_factor", 0, N, "coat_weight"),
    ("coat_color", DEFAULT_W_1, N, "coat_color"),
    ("coat_roughness", 0, N, "coat_roughness"),
    ("coat_anisotropy", 0, N, "coat_roughness_anisotropy"),
    ("coat_ior", 0, N, "coat_ior"),
    ("coat_normal", 0, N, "coat_normal_map"),
    ("sheen_factor", 0, N, "fuzz_weight"),
    ("sheen_color", DEFAULT_W_1, N, "fuzz_color"),
    ("sheen_roughness", 0, N, "fuzz_roughness"),
    ("emission_factor", 0, N, "emission_weight"),
    ("emission_factor", MULTIPLY_VALUE, N, "emission_luminance"),
    ("emission_color", DEFAULT_W_1, N, "emission_color"),
    ("thin_film_factor", 0, N, "thin_film_weight"),
    ("thin_film_thickness", 0, N, "thin_film_thickness"),
    ("thin_film_ior", 0, N, "thin_film_ior"),
    ("normal_map", 0, N, "bump"),
    ("normal_map", 0, N, "bump_map_amt"),
    ("displacement_map", 0, N, "displacement"),
    ("displacement_map", 0, N, "displacement_map_amt"),
    ("coat_normal", 0, N, "coat_bump"),
    ("coat_normal", 0, N, "coat_bump_map_amt"),
    ("tangent_map", 0, N, "geometry_tangent_map"),
    ("opacity", WIDEN_TO_RGB, N, "geometry_opacity"),
];

/// `ufbxi_openpbr_material_features` (ufbx 0.23.1).
const OPENPBR_FEATURES: &[Mapping] = &[("thin_walled", 0, N, "geometry_thin_walled")];

/// ufbx 0.23.1 `ufbxi_osl_standard_shader_pbr_mapping`, ufbx.c:19521-19626.
const OSL_MAPPING: &[Mapping] = &[
    ("base_factor", 0, N, "base"),
    ("base_color", DEFAULT_W_1, N, "base_color"),
    ("roughness", 0, N, "specular_roughness"),
    ("diffuse_roughness", 0, N, "diffuse_roughness"),
    ("metalness", 0, N, "metalness"),
    ("specular_factor", 0, N, "specular"),
    ("specular_color", DEFAULT_W_1, N, "specular_color"),
    ("specular_ior", 0, N, "specular_IOR"),
    ("specular_anisotropy", 0, N, "specular_anisotropy"),
    ("specular_rotation", 0, N, "specular_rotation"),
    ("transmission_factor", 0, N, "transmission"),
    ("transmission_color", DEFAULT_W_1, N, "transmission_color"),
    ("transmission_depth", 0, N, "transmission_depth"),
    (
        "transmission_scatter",
        WIDEN_TO_RGB,
        N,
        "transmission_scatter",
    ),
    (
        "transmission_scatter_anisotropy",
        0,
        N,
        "transmission_scatter_anisotropy",
    ),
    ("transmission_dispersion", 0, N, "transmission_dispersion"),
    (
        "transmission_extra_roughness",
        0,
        N,
        "transmission_extra_roughness",
    ),
    ("subsurface_factor", 0, N, "subsurface"),
    ("subsurface_color", DEFAULT_W_1, N, "subsurface_color"),
    ("subsurface_radius", WIDEN_TO_RGB, N, "subsurface_radius"),
    ("subsurface_scale", 0, N, "subsurface_scale"),
    ("subsurface_anisotropy", 0, N, "subsurface_anisotropy"),
    ("sheen_factor", 0, N, "sheen"),
    ("sheen_color", DEFAULT_W_1, N, "sheen_color"),
    ("sheen_roughness", 0, N, "sheen_roughness"),
    ("coat_factor", 0, N, "coat"),
    ("coat_color", DEFAULT_W_1, N, "coat_color"),
    ("coat_roughness", 0, N, "coat_roughness"),
    ("coat_ior", 0, N, "coat_IOR"),
    ("coat_anisotropy", 0, N, "coat_anisotropy"),
    ("coat_rotation", 0, N, "coat_rotation"),
    ("coat_normal", 0, N, "coat_normal"),
    (
        "coat_affect_base_color",
        DEFAULT_W_1,
        N,
        "coat_affect_color",
    ),
    ("coat_affect_base_roughness", 0, N, "coat_affect_roughness"),
    ("thin_film_thickness", 0, N, "thin_film_thickness"),
    ("thin_film_ior", 0, N, "thin_film_IOR"),
    ("emission_factor", 0, N, "emission"),
    ("emission_color", DEFAULT_W_1, N, "emission_color"),
    ("opacity", WIDEN_TO_RGB, N, "opacity"),
    ("normal_map", 0, N, "NormalMap"),
    ("normal_map", 0, N, "normalCamera"),
    ("tangent_map", 0, N, "tangent"),
];
/// ufbx 0.23.1 `ufbxi_osl_standard_shader_features`, ufbx.c:19521-19626.
const OSL_FEATURES: &[Mapping] = &[("thin_walled", 0, N, "thin_walled")];
/// ufbx 0.23.1 `ufbxi_arnold_shader_pbr_mapping`, ufbx.c:19521-19626.
const ARNOLD_MAPPING: &[Mapping] = &[
    ("base_factor", 0, N, "base"),
    ("base_color", DEFAULT_W_1, N, "baseColor"),
    ("roughness", 0, N, "specularRoughness"),
    ("diffuse_roughness", 0, N, "diffuseRoughness"),
    ("metalness", 0, N, "metalness"),
    ("specular_factor", 0, N, "specular"),
    ("specular_color", DEFAULT_W_1, N, "specularColor"),
    ("specular_ior", 0, N, "specularIOR"),
    ("specular_anisotropy", 0, N, "specularAnisotropy"),
    ("specular_rotation", 0, N, "specularRotation"),
    ("transmission_factor", 0, N, "transmission"),
    ("transmission_color", DEFAULT_W_1, N, "transmissionColor"),
    ("transmission_depth", 0, N, "transmissionDepth"),
    (
        "transmission_scatter",
        WIDEN_TO_RGB,
        N,
        "transmissionScatter",
    ),
    (
        "transmission_scatter_anisotropy",
        0,
        N,
        "transmissionScatterAnisotropy",
    ),
    ("transmission_dispersion", 0, N, "transmissionDispersion"),
    (
        "transmission_extra_roughness",
        0,
        N,
        "transmissionExtraRoughness",
    ),
    ("subsurface_factor", 0, N, "subsurface"),
    ("subsurface_color", DEFAULT_W_1, N, "subsurfaceColor"),
    ("subsurface_radius", WIDEN_TO_RGB, N, "subsurfaceRadius"),
    ("subsurface_scale", 0, N, "subsurfaceScale"),
    ("subsurface_anisotropy", 0, N, "subsurfaceAnisotropy"),
    ("sheen_factor", 0, N, "sheen"),
    ("sheen_color", DEFAULT_W_1, N, "sheenColor"),
    ("sheen_roughness", 0, N, "sheenRoughness"),
    ("coat_factor", 0, N, "coat"),
    ("coat_color", DEFAULT_W_1, N, "coatColor"),
    ("coat_roughness", 0, N, "coatRoughness"),
    ("coat_ior", 0, N, "coatIOR"),
    ("coat_anisotropy", 0, N, "coatAnisotropy"),
    ("coat_rotation", 0, N, "coatRotation"),
    ("coat_normal", 0, N, "coatNormal"),
    ("thin_film_thickness", 0, N, "thinFilmThickness"),
    ("thin_film_ior", 0, N, "thinFilmIOR"),
    ("emission_factor", 0, N, "emission"),
    ("emission_color", DEFAULT_W_1, N, "emissionColor"),
    ("opacity", WIDEN_TO_RGB, N, "opacity"),
    ("indirect_diffuse", 0, N, "indirectDiffuse"),
    ("indirect_specular", 0, N, "indirectSpecular"),
    ("normal_map", 0, N, "NormalMap"),
    ("normal_map", 0, N, "normalCamera"),
    ("tangent_map", 0, N, "tangent"),
    ("matte_color", DEFAULT_W_1, N, "aiMatteColor"),
    ("matte_factor", 0, N, "aiMatteColorA"),
    ("subsurface_type", 0, N, "subsurfaceType"),
    ("transmission_priority", 0, N, "dielectricPriority"),
    ("transmission_enable_in_aov", 0, N, "transmitAovs"),
];
/// ufbx 0.23.1 `ufbxi_arnold_shader_features`, ufbx.c:19521-19626.
const ARNOLD_FEATURES: &[Mapping] = &[
    ("matte", 0, N, "aiEnableMatte"),
    ("thin_walled", 0, N, "thinWalled"),
    ("caustics", 0, N, "caustics"),
    ("internal_reflections", 0, N, "internalReflections"),
    ("exit_to_background", 0, N, "exitToBackground"),
];

/// `ufbx_shader_type` values used here.
const SHADER_OSL: i64 = 3;
const SHADER_ARNOLD: i64 = 4;
const SHADER_UNKNOWN: i64 = 0;
const SHADER_FBX_LAMBERT: i64 = 1;
const SHADER_FBX_PHONG: i64 = 2;
const SHADER_3DS_MAX_PHYSICAL_MATERIAL: i64 = 5;
const SHADER_3DS_MAX_PBR_METAL_ROUGH: i64 = 6;
const SHADER_3DS_MAX_PBR_SPEC_GLOSS: i64 = 7;
const SHADER_GLTF_MATERIAL: i64 = 8;
const SHADER_OPENPBR_MATERIAL: i64 = 9;

struct ShaderMapping {
    maps: &'static [Mapping],
    features: &'static [Mapping],
    defaults: &'static [&'static str],
    prefix: &'static [u8],
    texture_suffix: &'static [u8],
    enabled_suffix: &'static [u8],
}
/// `ufbxi_shader_pbr_mappings[shader_type]` for the ported shader types.
fn pbr_mapping(shader_type: i64) -> Option<ShaderMapping> {
    let mut m = ShaderMapping {
        maps: &[],
        features: &[],
        defaults: &[],
        prefix: b"",
        texture_suffix: b"",
        enabled_suffix: b"",
    };
    match shader_type {
        SHADER_OSL | SHADER_ARNOLD => {
            m.maps = if shader_type == SHADER_OSL {
                OSL_MAPPING
            } else {
                ARNOLD_MAPPING
            };
            m.features = if shader_type == SHADER_OSL {
                OSL_FEATURES
            } else {
                ARNOLD_FEATURES
            };
            m.defaults = &[
                "pbr",
                "metalness",
                "diffuse",
                "specular",
                "coat",
                "sheen",
                "transmission",
                "opacity",
                "ior",
                "diffuse_roughness",
            ];
        }
        SHADER_UNKNOWN | SHADER_FBX_PHONG => {
            m.maps = PHONG_PBR_MAPPING;
            m.defaults = &["diffuse", "specular", "emission", "transmission"];
        }
        SHADER_FBX_LAMBERT => {
            m.maps = LAMBERT_PBR_MAPPING;
            m.defaults = &["diffuse", "emission", "transmission"];
        }
        SHADER_3DS_MAX_PHYSICAL_MATERIAL => {
            m.maps = PHYSICAL_PBR_MAPPING;
            m.features = PHYSICAL_FEATURES;
            m.defaults = &[
                "pbr",
                "metalness",
                "diffuse",
                "coat",
                "sheen",
                "transmission",
                "opacity",
                "ior",
            ];
            m.prefix = b"3dsMax|Parameters|";
            m.texture_suffix = b"_map";
            m.enabled_suffix = b"_map_on";
        }
        SHADER_3DS_MAX_PBR_METAL_ROUGH | SHADER_3DS_MAX_PBR_SPEC_GLOSS => {
            m.maps = if shader_type == SHADER_3DS_MAX_PBR_METAL_ROUGH {
                METAL_ROUGH_PBR_MAPPING
            } else {
                SPEC_GLOSS_PBR_MAPPING
            };
            m.features = MAX_PBR_FEATURES;
            m.defaults = if shader_type == SHADER_3DS_MAX_PBR_METAL_ROUGH {
                &["pbr", "metalness", "diffuse", "opacity"]
            } else {
                &["pbr", "specular", "diffuse", "opacity"]
            };
            m.prefix = b"3dsMax|main|";
            m.texture_suffix = b"_map";
        }
        SHADER_GLTF_MATERIAL => {
            m.maps = GLTF_PBR_MAPPING;
            m.features = GLTF_FEATURES;
            m.defaults = &[
                "pbr",
                "metalness",
                "diffuse",
                "emission",
                "opacity",
                "ambient_occlusion",
            ];
            m.prefix = b"3dsMax|";
            m.texture_suffix = b"Map";
        }
        SHADER_OPENPBR_MATERIAL => {
            m.maps = OPENPBR_MAPPING;
            m.features = OPENPBR_FEATURES;
            m.defaults = &[
                "pbr",
                "metalness",
                "diffuse",
                "specular",
                "coat",
                "sheen",
                "transmission",
                "opacity",
                "ior",
                "diffuse_roughness",
            ];
            m.prefix = b"3dsMax|Parameters|";
            m.texture_suffix = b"_map";
            m.enabled_suffix = b"_map_on";
        }
        _ => return None,
    }
    Some(m)
}

/// `ufbx_material_map`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Map {
    pub(crate) value: [f64; 4],
    pub(crate) int: i64,
    /// Texture element index.
    pub(crate) texture: Option<usize>,
    pub(crate) has_value: bool,
    pub(crate) texture_enabled: bool,
    pub(crate) feature_disabled: bool,
    pub(crate) components: u64,
}
impl Map {
    fn to_v(&self) -> V {
        V::Obj(
            Obj::new()
                .with("value_vec4", V::Vec(self.value.to_vec()))
                .with("value_int", V::I(self.int))
                .with("texture", V::Ref(self.texture))
                .with("has_value", V::Bool(self.has_value))
                .with("texture_enabled", V::Bool(self.texture_enabled))
                .with("feature_disabled", V::Bool(self.feature_disabled))
                .with("value_components", V::U(self.components)),
        )
    }
}

/// `ufbx_material_texture`.
#[derive(Debug, Clone)]
pub(crate) struct MaterialTexture {
    pub(crate) material_prop: Vec<u8>,
    pub(crate) shader_prop: Vec<u8>,
    pub(crate) texture: usize,
}

/// File name fields shared by videos, textures and texture files.
#[derive(Debug, Clone, Default)]
struct Names {
    filename: Vec<u8>,
    absolute: Vec<u8>,
    relative: Vec<u8>,
    raw_filename: Vec<u8>,
    raw_absolute: Vec<u8>,
    raw_relative: Vec<u8>,
}
impl Names {
    fn set(&self, obj: &mut Obj) {
        obj.set("filename", s(&self.filename));
        obj.set("absolute_filename", s(&self.absolute));
        obj.set("relative_filename", s(&self.relative));
        obj.set("raw_filename", s(&self.raw_filename));
        obj.set("raw_absolute_filename", s(&self.raw_absolute));
        obj.set("raw_relative_filename", s(&self.raw_relative));
    }
}

/// One element of the materials domain as the projection sees it.
pub struct Item<'p> {
    /// Element index in canonical order.
    pub element: usize,
    pub ty: ElementType,
    /// Source DOM node.
    pub node: usize,
    pub props: PropertyLayers<'p>,
    /// `connections_dst` in ufbx order: (source element, dst prop).
    pub dst: Vec<(usize, Vec<u8>)>,
    /// `connections_src` in ufbx order: destination elements.
    pub src: Vec<usize>,
}

/// Scene facts the projection depends on.
pub struct Env<'a, 'd> {
    pub doc: &'a Document<'d>,
    pub(crate) allocation: &'a Budget,
    pub children: &'a crate::scene_property::PropertyIndex,
    pub ascii: bool,
    /// `metadata.original_file_path` (`DocumentUrl`): UTF-8 view and raw bytes.
    pub original_file_path: (Vec<u8>, Vec<u8>),
    /// Element type by canonical element index.
    pub types: &'a [ElementType],
    /// Lean projection (public `load`): fill [`Output::materials`] and
    /// [`Output::textures`] instead of records and texture files.
    pub lean: bool,
}

/// Records to merge into element objects, plus `scene.texture_files`; the
/// typed results instead in a lean projection.
pub struct Output {
    pub records: Vec<(usize, Obj)>,
    pub(crate) num_shader_textures: u64,
    pub texture_files: Vec<V>,
    /// Typed materials by element index (lean projection only).
    pub(crate) materials: Vec<(usize, LeanMaterial)>,
    /// Typed textures by element index (lean projection only).
    pub(crate) textures: Vec<(usize, LeanTexture)>,
    /// Conversion warnings (content that is read but not ported).
    pub warnings: Vec<String>,
}

/// The material values the public `Scene` exposes.
pub(crate) struct LeanMaterial {
    pub(crate) features: Vec<(bool, bool)>,
    pub(crate) properties: Vec<crate::SourceProperty>,
    pub(crate) attachments: Vec<MaterialTexture>,
    pub(crate) shading_model: Vec<u8>,
    /// In [`FBX_MAPS`] order.
    pub(crate) fbx: Vec<Map>,
    /// In [`PBR_MAPS`] order.
    pub(crate) pbr: Vec<Map>,
}

/// The texture values the public `Scene` exposes.
pub(crate) struct LeanTexture {
    pub(crate) layered: bool,
    pub(crate) layers: Vec<(usize, i64, f64)>,
    pub(crate) file_textures: Vec<usize>,
    pub(crate) filename: Vec<u8>,
    pub(crate) relative_filename: Vec<u8>,
    pub(crate) absolute_filename: Vec<u8>,
    pub(crate) content: Vec<u8>,
    pub(crate) uv_set: Vec<u8>,
    pub(crate) wrap: [i64; 2],
}

impl Env<'_, '_> {
    fn child(&self, node: usize, name: &[u8]) -> Option<usize> {
        self.children
            .children(node)
            .find(|&c| self.doc.nodes[c].name == name)
    }
    /// `ufbxi_find_val1(node, name, "S")`: assigns only when present.
    fn val_str(&self, node: usize, name: &[u8], dst: &mut Vec<u8>) -> Result<(), Error> {
        if let Some(c) = self.child(node, name) {
            if let Some((Some(view), _)) = node_string(self.doc, c, self.ascii, self.allocation)? {
                *dst = view;
            }
        }
        Ok(())
    }
    /// `ufbxi_find_val1(node, name, "b")`: assigns only when present.
    fn val_blob(&self, node: usize, name: &[u8], dst: &mut Vec<u8>) -> Result<(), Error> {
        if let Some(c) = self.child(node, name) {
            if let Some((_, raw)) = node_string(self.doc, c, self.ascii, self.allocation)? {
                *dst = raw;
            }
        }
        Ok(())
    }
    /// The four file name reads shared by `ufbxi_read_texture` and
    /// `ufbxi_read_video`.
    fn read_names(&self, node: usize) -> Result<Names, Error> {
        let mut n = Names::default();
        self.val_str(node, b"FileName", &mut n.absolute)?;
        self.val_str(node, b"Filename", &mut n.absolute)?;
        self.val_str(node, b"RelativeFileName", &mut n.relative)?;
        self.val_str(node, b"RelativeFilename", &mut n.relative)?;
        self.val_blob(node, b"FileName", &mut n.raw_absolute)?;
        self.val_blob(node, b"Filename", &mut n.raw_absolute)?;
        self.val_blob(node, b"RelativeFileName", &mut n.raw_relative)?;
        self.val_blob(node, b"RelativeFilename", &mut n.raw_relative)?;
        Ok(n)
    }
    /// `ufbxi_resolve_filenames` for the string and the raw views.
    fn resolve(&self, n: &mut Names) -> Result<(), Error> {
        if n.relative.is_empty() {
            absolute_to_relative(
                &mut n.relative,
                &self.original_file_path.0,
                &n.absolute,
                self.allocation,
            )?;
        }
        n.filename = resolve_relative_filename(&n.relative, self.allocation)?;
        if n.raw_relative.is_empty() {
            absolute_to_relative(
                &mut n.raw_relative,
                &self.original_file_path.1,
                &n.raw_absolute,
                self.allocation,
            )?;
        }
        n.raw_filename = resolve_relative_filename(&n.raw_relative, self.allocation)?;
        Ok(())
    }
}

fn is_separator(c: u8) -> bool {
    c == b'/' || c == b'\\'
}
/// `ufbxi_next_path_segment`.
fn next_path_segment(data: &[u8], begin: usize, length: usize) -> usize {
    (begin..length)
        .find(|&i| is_separator(data[i]))
        .unwrap_or(length)
}

/// `ufbxi_absolute_to_relative_path`: `src` relative to the directory of
/// `rel`, written to `dst` only when both are non-empty and share a root.
fn absolute_to_relative(
    dst: &mut Vec<u8>,
    rel: &[u8],
    src: &[u8],
    a: &Budget,
) -> Result<(), Error> {
    if rel.is_empty() || src.is_empty() || rel[0] != src[0] {
        return Ok(());
    }
    let mut rel_length = rel.len();
    while rel_length > 0 && !is_separator(rel[rel_length - 1]) {
        rel_length -= 1;
    }
    if rel_length == 0 {
        return Ok(());
    }
    let separator = rel[rel_length - 1];
    let src_length = src.len();
    let (mut rel_begin, mut src_begin) = (0, 0);
    while rel_begin < rel_length && src_begin < src_length {
        let rel_end = next_path_segment(rel, rel_begin, rel_length);
        let src_end = next_path_segment(src, src_begin, src_length);
        if rel_end != src_end || rel[rel_begin..rel_end] != src[src_begin..src_end] {
            break;
        }
        rel_begin = rel_end + 1;
        src_begin = src_end + 1;
    }
    let mut out = Vec::new();
    while rel_begin < rel_length {
        let rel_end = next_path_segment(rel, rel_begin, rel_length);
        a.reserve(&mut out, 3)?;
        out.extend_from_slice(&[b'.', b'.', separator]);
        rel_begin = rel_end + 1;
    }
    while src_begin < src_length {
        let src_end = next_path_segment(src, src_begin, src_length);
        a.reserve(&mut out, src_end - src_begin)?;
        out.extend_from_slice(&src[src_begin..src_end]);
        if src_end < src_length {
            a.push(&mut out, separator)?;
        }
        src_begin = src_end + 1;
    }
    *dst = out;
    Ok(())
}

/// `ufbxi_resolve_relative_filename` with the import profile's empty
/// `metadata.relative_root` (the scene is loaded from memory without a
/// file name, so the prefix is always empty).
fn resolve_relative_filename(src: &[u8], a: &Budget) -> Result<Vec<u8>, Error> {
    let start = src.iter().position(|&c| !is_separator(c));
    let Some(start) = start else {
        return Ok(Vec::new());
    };
    a.collect(
        src[start..]
            .iter()
            .map(|&c| if is_separator(c) { PATH_SEPARATOR } else { c }),
    )
}

fn find<'a, 'p>(props: &'a PropertyLayers<'p>, name: &[u8]) -> Option<&'a AuthoredProperty<'p>> {
    props.find(name)
}
/// `ufbxi_find_vec3`.
fn find_vec3(props: &PropertyLayers<'_>, name: &[u8], def: [f64; 3]) -> [f64; 3] {
    find(props, name).map_or(def, |p| [p.value_real[0], p.value_real[1], p.value_real[2]])
}
/// `ufbxi_find_int` / `ufbx_find_int`.
fn find_int(props: &PropertyLayers<'_>, name: &[u8], def: i64) -> i64 {
    find(props, name).map_or(def, |p| p.value_int)
}
/// `ufbxi_find_real`.
fn find_real(props: &PropertyLayers<'_>, name: &[u8], def: f64) -> f64 {
    find(props, name).map_or(def, |p| p.value_real[0])
}
/// `ufbxi_find_enum`.
fn find_enum(props: &PropertyLayers<'_>, name: &[u8], def: i64, max: i64) -> i64 {
    match find(props, name) {
        Some(p) if (0..=max).contains(&p.value_int) => p.value_int,
        _ => def,
    }
}

/// `ufbxi_get_texture_transform`.
fn texture_transform(props: &PropertyLayers<'_>) -> Transform {
    let scale_pivot = find_vec3(props, b"TextureScalingPivot", [0.0; 3]);
    let rot_pivot = find_vec3(props, b"TextureRotationPivot", [0.0; 3]);
    let translation = find_vec3(props, b"Translation", [0.0; 3]);
    let rotation = find_vec3(props, b"Rotation", [0.0; 3]);
    let scaling = find_vec3(props, b"Scaling", [1.0; 3]);
    let mut t = sn::IDENTITY_TRANSFORM;
    sn::sub_translate(&mut t, scale_pivot);
    sn::mul_scale(&mut t, scaling);
    sn::add_translate(&mut t, scale_pivot);
    sn::sub_translate(&mut t, rot_pivot);
    sn::mul_rotate(&mut t, rotation, 0);
    sn::add_translate(&mut t, rot_pivot);
    sn::add_translate(&mut t, translation);
    if find_int(props, b"UVSwap", 0) != 0 {
        sn::mul_scale(&mut t, [-1.0, 0.0, 0.0]);
        sn::mul_rotate(&mut t, [0.0, 0.0, -90.0], 0);
    }
    t
}

/// `ufbx_find_prop_texture_len` over textures sorted by `material_prop`.
fn prop_texture(textures: &[MaterialTexture], name: &[u8]) -> Option<usize> {
    let i = textures.partition_point(|t| t.material_prop.as_slice() < name);
    textures
        .get(i)
        .filter(|t| t.material_prop == name)
        .map(|t| t.texture)
}

// Fixed scratch names preserve the loader's fallible-allocation contract.
fn mapping_name<'a>(buf: &'a mut [u8; 512], prefix: &[u8], name: &str, suffix: &[u8]) -> &'a [u8] {
    let mut len = 0;
    for part in [prefix, name.as_bytes(), suffix] {
        buf[len..len + part.len()].copy_from_slice(part);
        len += part.len();
    }
    &buf[..len]
}
#[derive(Clone, Copy)]
enum Fetch {
    ValuesAndTextures,
    Textures,
    TextureEnabled,
}
fn implementation_type(props: &PropertyLayers<'_>) -> i64 {
    match find(props, b"RenderAPI").map(|p| p.value_str) {
        Some(b"ARNOLD_SHADER_ID") => SHADER_ARNOLD,
        Some(b"OSL") => SHADER_OSL,
        Some(b"SFX_PBS_SHADER") => 10,
        _ => SHADER_UNKNOWN,
    }
}
fn read_bindings(env: &Env<'_, '_>, node: usize, a: &Budget) -> Result<BindingTable, Error> {
    let mut entries = Vec::new();
    for child in env.children.children(node) {
        if env.doc.nodes[child].name != b"Entry" {
            continue;
        }
        let NodeData::Values(values) = node_data(env.doc, child, env.ascii, a)? else {
            continue;
        };
        use crate::canon_dom::Scalar;
        let [Scalar::Str(src), Scalar::Str(st), Scalar::Str(dst), Scalar::Str(dt), ..] =
            values.as_slice()
        else {
            continue;
        };
        let pair = if st == b"FbxPropertyEntry" && dt == b"FbxSemanticEntry" {
            Some((a.bytes(dst)?, a.bytes(src)?))
        } else if st == b"FbxSemanticEntry" && dt == b"FbxPropertyEntry" {
            Some((a.bytes(src)?, a.bytes(dst)?))
        } else {
            None
        };
        if let Some(pair) = pair {
            a.push(&mut entries, pair)?;
        }
    }
    a.stable_sort(&mut entries, |a, b| a.0.cmp(&b.0))?;
    Ok(entries)
}

type PropBinding = (Vec<u8>, Vec<u8>);
type BindingTable = Vec<PropBinding>;
type Bindings = Vec<BindingTable>;
// First table containing the semantic wins; duplicates retain their stable order.
fn bound_names<'a>(bindings: &'a [BindingTable], name: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
    let table = bindings
        .iter()
        .find(|t| t.iter().any(|(shader, _)| shader == name));
    let identity = if table.is_none() { Some(name) } else { None };
    table
        .into_iter()
        .flat_map(move |t| {
            t.iter()
                .filter(move |(shader, _)| shader == name)
                .map(|(_, material)| material.as_slice())
        })
        .chain(identity)
}
struct MappingSource<'a> {
    bindings: &'a [BindingTable],
    prefix: &'a [u8],
    suffix: &'a [u8],
    fetch: Fetch,
}
/// `ufbxi_fetch_mapping_maps`, ufbx.c:19968-20129; semantic bindings
/// fall back to identity only when no table contains that semantic.
fn fetch_mapping_maps(
    props: &PropertyLayers<'_>,
    textures: &[MaterialTexture],
    maps: &mut [Map],
    index: fn(&str) -> usize,
    mappings: &[Mapping],
    source: MappingSource<'_>,
) {
    for &(target, flags, transform, name) in mappings {
        let map = &mut maps[index(target)];
        let mut scratch = [0; 512];
        let name = mapping_name(&mut scratch, source.prefix, name, source.suffix);
        for name in bound_names(source.bindings, name) {
            let prop = find(props, name);
            if let Some(p) = prop.filter(|p| {
                p.kind != Type::Reference && matches!(source.fetch, Fetch::ValuesAndTextures)
            }) {
                if flags & MULTIPLY_VALUE != 0 {
                    map.value[0] *= p.value_real[0];
                    map.int = crate::canon_dom::f64_to_i64(map.value[0]);
                } else {
                    map.value = p.value_real;
                    map.int = p.value_int;
                }
                map.has_value = true;
                if transform == MatTransform::UnknownShininess {
                    // ufbxi_mat_transform_unknown_shininess
                    let v = &mut map.value[0];
                    if *v >= 0.0 {
                        *v = 1.0 - v.sqrt() * 0.1;
                    }
                    // `!(x >= 0)`: negative or NaN.
                    if v.is_nan() || *v < 0.0 {
                        *v = 0.0;
                    }
                }
                if flags & DEFAULT_W_1 != 0 && p.flags & PROP_FLAG_VALUE_VEC4 == 0 {
                    map.value[3] = 1.0;
                }
                if flags & WIDEN_TO_RGB != 0 && p.flags & PROP_FLAG_VALUE_REAL != 0 {
                    map.value[1] = map.value[0];
                    map.value[2] = map.value[0];
                }
                map.components = if p.flags & PROP_FLAG_VALUE_REAL != 0 {
                    1
                } else if p.flags & PROP_FLAG_VALUE_VEC2 != 0 {
                    2
                } else if p.flags & PROP_FLAG_VALUE_VEC3 != 0 {
                    3
                } else if p.flags & PROP_FLAG_VALUE_VEC4 != 0 {
                    4
                } else {
                    0
                };
            }
            if matches!(source.fetch, Fetch::TextureEnabled) {
                if let Some(p) = prop {
                    map.texture_enabled = p.value_int != 0;
                }
            } else if let Some(t) = prop_texture(textures, name) {
                map.texture = Some(t);
                map.texture_enabled = true;
            }
        }
    }
}

/// `ufbxi_update_factor`.
fn update_factor(maps: &mut [Map], factor: usize, color: usize) {
    if maps[factor].has_value {
        return;
    }
    let c = &maps[color];
    let on = c.has_value && c.value != [0.0; 4];
    let f = &mut maps[factor];
    f.value[0] = if on { 1.0 } else { 0.0 };
    f.int = i64::from(on);
}

/// `ufbxi_fetch_maps`: (fbx maps, pbr maps, features as (enabled, explicit)).
type MaterialMaps = (Vec<Map>, Vec<Map>, Vec<(bool, bool)>);
fn fetch_maps(
    key: &str,
    shader_type: i64,
    props: &PropertyLayers<'_>,
    textures: &[MaterialTexture],
    bindings: &Bindings,
    warnings: &mut Vec<String>,
    a: &Budget,
) -> Result<MaterialMaps, Error> {
    let mut fbx_maps = a.filled(FBX_MAPS.len(), Map::default())?;
    let mut pbr_maps = a.filled(PBR_MAPS.len(), Map::default())?;
    let mut features = a.filled(FEATURES.len(), (false, false))?;
    fetch_mapping_maps(
        props,
        textures,
        &mut fbx_maps,
        fbx,
        BASE_FBX_MAPPING,
        MappingSource {
            bindings: &[],
            prefix: b"",
            suffix: b"",
            fetch: Fetch::ValuesAndTextures,
        },
    );
    match pbr_mapping(shader_type) {
        Some(m) => {
            for f in m.defaults {
                features[feature(f)].0 = true;
            }
            if !m.texture_suffix.is_empty() {
                fetch_mapping_maps(
                    props,
                    textures,
                    &mut pbr_maps,
                    pbr,
                    m.maps,
                    MappingSource {
                        bindings,
                        prefix: m.prefix,
                        suffix: m.texture_suffix,
                        fetch: Fetch::Textures,
                    },
                );
            }
            fetch_mapping_maps(
                props,
                textures,
                &mut pbr_maps,
                pbr,
                m.maps,
                MappingSource {
                    bindings,
                    prefix: m.prefix,
                    suffix: b"",
                    fetch: Fetch::ValuesAndTextures,
                },
            );
            if !m.enabled_suffix.is_empty() {
                fetch_mapping_maps(
                    props,
                    textures,
                    &mut pbr_maps,
                    pbr,
                    m.maps,
                    MappingSource {
                        bindings,
                        prefix: m.prefix,
                        suffix: m.enabled_suffix,
                        fetch: Fetch::TextureEnabled,
                    },
                );
            }
            for &(target, flags, _, name) in m.features {
                let mut scratch = [0; 512];
                let name = mapping_name(&mut scratch, m.prefix, name, b"");
                for name in bound_names(bindings, name) {
                    if let Some(p) = find(props, name).filter(|p| p.kind != Type::Reference) {
                        let mut enabled = p.value_int != 0;
                        if flags & FEATURE_INVERTED != 0 {
                            enabled = !enabled;
                        }
                        if flags & FEATURE_IF_AROUND_1 != 0 {
                            enabled = (0.5..=1.5).contains(&p.value_real[0]);
                        }
                        features[feature(target)] = (enabled, true);
                    }
                }
            }
        }
        None => a.push(
            warnings,
            a.format(format_args!(
                "{key}: PBR mapping for shader type {shader_type} is not ported"
            ))?,
        )?,
    }
    for (f, c) in [
        ("diffuse_factor", "diffuse_color"),
        ("specular_factor", "specular_color"),
        ("reflection_factor", "reflection_color"),
        ("transparency_factor", "transparency_color"),
        ("emission_factor", "emission_color"),
        ("ambient_factor", "ambient_color"),
    ] {
        update_factor(&mut fbx_maps, fbx(f), fbx(c));
    }
    for (f, c) in [
        ("base_factor", "base_color"),
        ("specular_factor", "specular_color"),
        ("emission_factor", "emission_color"),
        ("sheen_factor", "sheen_color"),
        ("thin_film_factor", "thin_film_thickness"),
        ("transmission_factor", "transmission_color"),
    ] {
        update_factor(&mut pbr_maps, pbr(f), pbr(c));
    }
    let (tr, r, ter) = (
        pbr("transmission_roughness"),
        pbr("roughness"),
        pbr("transmission_extra_roughness"),
    );
    if !pbr_maps[tr].has_value && pbr_maps[r].has_value && pbr_maps[ter].has_value {
        pbr_maps[tr].value[0] = pbr_maps[r].value[0] + pbr_maps[ter].value[0];
    }
    for (f, rough, gloss) in [
        ("roughness_as_glossiness", "roughness", "glossiness"),
        (
            "coat_roughness_as_glossiness",
            "coat_roughness",
            "coat_glossiness",
        ),
        (
            "transmission_roughness_as_glossiness",
            "transmission_roughness",
            "transmission_glossiness",
        ),
    ] {
        let (rough, gloss) = (pbr(rough), pbr(gloss));
        if features[feature(f)].0 {
            pbr_maps[gloss] = pbr_maps[rough].clone();
            pbr_maps[rough] = Map::default();
            if pbr_maps[gloss].has_value {
                pbr_maps[rough].value[0] = 1.0 - pbr_maps[gloss].value[0];
            }
        } else if pbr_maps[rough].has_value {
            pbr_maps[gloss].value[0] = 1.0 - pbr_maps[rough].value[0];
        }
    }
    Ok((fbx_maps, pbr_maps, features))
}

/// A `UFBX_LIST_TYPE` value.
fn list(items: impl Iterator<Item = V>) -> V {
    V::Obj(Obj::new().with("data", V::List(items.collect())))
}

fn maps_obj(names: &[&'static str], maps: &[Map]) -> V {
    let mut o = Obj::new();
    for (&n, m) in names.iter().zip(maps) {
        o.set(n, m.to_v());
    }
    V::Obj(o)
}

// ufbxi_shader_texture_find_prefix / ufbxi_finalize_shader_texture,
// ufbx.c:20430-20492, 20560-20684 (unknown procedural shader branch).
type ShaderInput<'a, 'p> = (
    &'a AuthoredProperty<'p>,
    Option<&'a AuthoredProperty<'p>>,
    Option<&'a AuthoredProperty<'p>>,
);
fn shader_texture(
    it: &Item<'_>,
    env: &Env<'_, '_>,
    a: &Budget,
) -> Result<(Option<Obj>, Vec<usize>), Error> {
    let props = &it.props;
    let authored = &props.authored;
    let mut prefix = Vec::new();
    let named = find(props, b"3dsMax|params|OSLShaderName").map_or(&b""[..], |p| p.value_str);
    for suffix in [
        b" Parameters/Connections".as_slice(),
        named,
        b"3dsMax|parameters".as_slice(),
    ] {
        if suffix.is_empty() {
            continue;
        }
        if let Some(p) = authored
            .iter()
            .find(|p| p.kind == Type::Compound && p.name.ends_with(suffix))
        {
            prefix = a.bytes(p.name)?;
            if !prefix.ends_with(b"|") {
                a.push(&mut prefix, b'|')?;
            }
            break;
        }
    }
    if prefix.is_empty() {
        for suffix in [
            b" Parameters/Connections".as_slice(),
            named,
            b"3dsMax|parameters".as_slice(),
        ] {
            if suffix.is_empty() {
                continue;
            }
            if let Some(name) = authored
                .iter()
                .filter_map(|p| {
                    p.name
                        .iter()
                        .rposition(|&b| b == b'|')
                        .map(|i| (&p.name[..i], &p.name[i + 1..]))
                })
                .map(|(n, _)| n)
                .find(|n| n.ends_with(suffix))
            {
                prefix = a.bytes(name)?;
                a.push(&mut prefix, b'|')?;
                break;
            }
        }
    }
    if prefix.is_empty() {
        return Ok((None, Vec::new()));
    }
    let derived = prefix
        .strip_suffix(b" Parameters/Connections|")
        .map(|n| n.rsplit(|&b| b == b'|').next().unwrap_or(n));
    let name = if !named.is_empty() {
        named
    } else {
        derived
            .unwrap_or_else(|| find(props, b"3dsMax|MaxTexture").map_or(&b""[..], |p| p.value_str))
    };
    let mut inputs: Vec<ShaderInput<'_, '_>> = Vec::new();
    for p in authored.iter().filter(|p| p.name.starts_with(&prefix)) {
        let name = &p.name[prefix.len()..];
        let modifier = [
            (b"_map".as_slice(), false),
            (b".shader".as_slice(), false),
            (b".connected".as_slice(), true),
            (b"Enabled".as_slice(), true),
        ]
        .iter()
        .find_map(|(suffix, enabled)| name.strip_suffix(*suffix).map(|base| (base, *enabled)));
        if let Some((base, enabled)) = modifier {
            if let Ok(i) = inputs.binary_search_by(|p| p.0.name[prefix.len()..].cmp(base)) {
                let input = &mut inputs[i];
                if enabled {
                    input.2 = Some(p);
                } else {
                    input.1 = Some(p);
                }
                continue;
            }
        }
        a.push(&mut inputs, (p, None, None))?;
    }
    let texture = |p: &AuthoredProperty<'_>| {
        let first = it.dst.partition_point(|(_, name)| name.as_slice() < p.name);
        it.dst[first..]
            .iter()
            .take_while(|(_, name)| name.as_slice() == p.name)
            .find(|(s, _)| env.types[*s] == ElementType::Texture)
            .map(|(s, _)| *s)
    };
    let mut deps = Vec::new();
    for (p, tp, _) in &inputs {
        if let Some(tex) = tp.and_then(texture).or_else(|| texture(p)) {
            a.push(&mut deps, tex)?;
        }
    }
    if env.lean {
        return Ok((Some(Obj::new()), deps));
    }
    let records = a.try_collect(inputs.iter().map(|(p, tp, ep)| {
        let tex = tp.and_then(texture).or_else(|| texture(p));
        Ok(V::Obj(
            Obj::new()
                .with("name", s(&p.name[prefix.len()..]))
                .with("prop", crate::canon_scene::prop_obj(p))
                .with(
                    "texture_prop",
                    tp.map_or(V::Null, crate::canon_scene::prop_obj),
                )
                .with(
                    "texture_enabled_prop",
                    ep.map_or(V::Null, crate::canon_scene::prop_obj),
                )
                .with("value_vec4", V::Vec(p.value_real.to_vec()))
                .with("value_int", V::I(p.value_int))
                .with("value_str", s(p.value_str))
                .with("value_blob", s(p.value_blob))
                .with("texture", V::Ref(tex))
                .with(
                    "texture_enabled",
                    V::Bool(ep.map_or(tex.is_some(), |p| p.value_int != 0)),
                ),
        ))
    }))?;
    let classid = u64::from(crate::numeric::low_word(find_int(
        props,
        b"3dsMax|ClassIDa",
        0,
    ))) << 32
        | u64::from(crate::numeric::low_word(find_int(
            props,
            b"3dsMax|ClassIDb",
            0,
        )));
    let source = find(props, b"3dsMax|params|OSLCode");
    Ok((
        Some(
            Obj::new()
                .with("type", V::I(0))
                .with("shader_name", s(name))
                .with("shader_type_id", V::U(classid))
                .with("shader_source", s(source.map_or(&[], |p| p.value_str)))
                .with("raw_shader_source", s(source.map_or(&[], |p| p.value_blob)))
                .with("prop_prefix", s(&prefix))
                .with("inputs", V::Obj(Obj::new().with("data", V::List(records)))),
        ),
        deps,
    ))
}

/// `ufbx_texture` state during finalization.
struct Texture {
    shader: Option<Obj>,
    shader_dependencies: Vec<usize>,
    item: usize,
    layered: bool,
    names: Names,
    content: Vec<u8>,
    video: Option<usize>,
    file_index: u64,
    has_file: bool,
    /// (texture index, blend mode, alpha).
    layers: Vec<(usize, i64, f64)>,
    file_textures: Vec<usize>,
}

/// Project every material, texture and video, in ufbx's typed order.
pub fn project(
    env: &Env<'_, '_>,
    items: &[Item<'_>],
    allocation_limit: usize,
) -> Result<Output, crate::Error> {
    let budget = crate::api_alloc::Budget::new(allocation_limit);
    let mut out = Output {
        records: Vec::new(),
        num_shader_textures: 0,
        texture_files: Vec::new(),
        materials: Vec::new(),
        textures: Vec::new(),
        warnings: Vec::new(),
    };
    let a = &budget;
    // Typed order is source order (element ordinal == DOM node + 1).
    let typed = |ty: ElementType| -> Result<Vec<usize>, Error> {
        let mut v: Vec<usize> = a.collect((0..items.len()).filter(|&i| items[i].ty == ty))?;
        a.stable_sort(&mut v, |&i, &j| items[i].node.cmp(&items[j].node))?;
        Ok(v)
    };
    let mut item_slots = a.filled(env.types.len(), None)?;
    for (i, item) in items.iter().enumerate() {
        item_slots[item.element] = Some(i);
    }
    let item = |element: usize| -> Result<&Item<'_>, Error> {
        item_slots
            .get(element)
            .and_then(|s| *s)
            .and_then(|i| items.get(i))
            .ok_or(Error::InvalidData("missing material shader element"))
    };
    let videos = typed(ElementType::Video)?;
    let textures = typed(ElementType::Texture)?;
    let materials = typed(ElementType::Material)?;
    let key = |i: usize| a.format(format_args!("element {}", items[i].element));

    // ufbxi_read_material and the first finalize pass.
    let mut mats = Vec::new();
    for &m in &materials {
        let it = &items[m];
        let mut shading_model = Vec::new();
        env.val_str(it.node, b"ShadingModel", &mut shading_model)?;
        let mut shader_type = match shading_model.as_slice() {
            b"lambert" | b"Lambert" => SHADER_FBX_LAMBERT,
            b"phong" | b"Phong" => SHADER_FBX_PHONG,
            _ => SHADER_UNKNOWN,
        };
        let mut prefix: &[u8] = b"";
        let shader = it
            .src
            .iter()
            .copied()
            .find(|&d| env.types[d] == ElementType::Shader);
        let mut bindings: Bindings = Vec::new();
        if let Some(shader) = shader {
            shader_type = implementation_type(&item(shader)?.props);
            for &(table, _) in &item(shader)?.dst {
                if env.types[table] != ElementType::ShaderBinding {
                    continue;
                }
                let entries = read_bindings(env, item(table)?.node, a)?;
                a.push(&mut bindings, entries)?;
            }
        } else if shader_type == SHADER_UNKNOWN {
            let a = crate::numeric::low_word(find_int(&it.props, b"3dsMax|ClassIDa", 0));
            let b = crate::numeric::low_word(find_int(&it.props, b"3dsMax|ClassIDb", 0));
            let known: Option<(i64, &[u8])> = match (a, b) {
                (0x3d6b_1cec, 0xdead_c001) => {
                    Some((SHADER_3DS_MAX_PHYSICAL_MATERIAL, b"3dsMax|Parameters|"))
                }
                (0xf155_1e33, 0x37fb_1337) => {
                    Some((SHADER_OPENPBR_MATERIAL, b"3dsMax|Parameters|"))
                }
                (0x3842_0192, 0x45fe_4e1b) => Some((SHADER_GLTF_MATERIAL, b"3dsMax|")),
                (0xd00f_1e00, 0xbe77_e500) => {
                    Some((SHADER_3DS_MAX_PBR_METAL_ROUGH, b"3dsMax|main|"))
                }
                (0xd00f_1e00, 0x01db_ad33) => {
                    Some((SHADER_3DS_MAX_PBR_SPEC_GLOSS, b"3dsMax|main|"))
                }
                _ => None,
            };
            if let Some((t, p)) = known {
                shader_type = t;
                prefix = p;
            }
        }
        // ufbxi_fetch_textures (no node search): connections without src prop.
        let tex: Vec<MaterialTexture> = a.try_collect(
            it.dst
                .iter()
                .filter(|(src, _)| env.types[*src] == ElementType::Texture)
                .map(|(src, prop)| {
                    Ok(MaterialTexture {
                        material_prop: a.bytes(prop)?,
                        shader_prop: a.bytes(prop)?,
                        texture: *src,
                    })
                }),
        )?;
        a.push(
            &mut mats,
            (
                m,
                shading_model,
                shader_type,
                a.bytes(prefix)?,
                tex,
                bindings,
            ),
        )?;
    }

    // ufbxi_resolve_file_content.
    let mut vids: Vec<(Names, Vec<u8>)> = a.try_collect(videos.iter().map(|&v| {
        let it = &items[v];
        let mut names = env.read_names(it.node)?;
        let content = env
            .child(it.node, b"Content")
            .map(|c| node_content(env.doc, c, env.ascii, env.allocation))
            .transpose()?
            .unwrap_or_default();
        env.resolve(&mut names)?;
        Ok((names, content))
    }))?;
    let mut providers = crate::fallible_map::Map::new();
    for (i, (names, content)) in vids.iter().enumerate() {
        if !content.is_empty() && !names.absolute.is_empty() {
            providers.try_get_or_insert_with(a.bytes(&names.absolute)?, a, || Ok(i))?;
        }
    }
    for i in 0..vids.len() {
        if vids[i].1.is_empty() {
            if let Some(&j) = providers.get(vids[i].0.absolute.as_slice()) {
                vids[i].1 = a.bytes(&vids[j].1)?;
            }
        }
    }

    // Texture finalize pass.
    let mut texs: Vec<Texture> = Vec::new();
    let mut tex_slot = a.filled(env.types.len(), None)?;
    for (k, &t) in textures.iter().enumerate() {
        tex_slot[items[t].element] = Some(k);
    }
    let mut video_slot = a.filled(env.types.len(), None)?;
    for (k, &v) in videos.iter().enumerate() {
        video_slot[items[v].element] = Some(k);
    }
    let tex_of = |element: usize| tex_slot[element];
    let mut file_keys = crate::fallible_map::Map::new();
    let mut files: Vec<Names> = Vec::new();
    let mut file_content: Vec<Vec<u8>> = Vec::new();
    for &t in &textures {
        let it = &items[t];
        let layered = env.doc.nodes[it.node].name == b"LayeredTexture";
        let mut names = if layered {
            Names::default()
        } else {
            env.read_names(it.node)?
        };
        let video = it
            .dst
            .iter()
            .filter(|(_, p)| p.is_empty())
            .map(|(s, _)| *s)
            .find(|&s| env.types[s] == ElementType::Video);
        let content = video
            .and_then(|v| video_slot[v])
            .map(|k| a.bytes(&vids[k].1))
            .transpose()?
            .unwrap_or_default();
        let classid = u64::from(crate::numeric::low_word(find_int(
            &it.props,
            b"3dsMax|ClassIDa",
            0,
        ))) << 32
            | u64::from(crate::numeric::low_word(find_int(
                &it.props,
                b"3dsMax|ClassIDb",
                0,
            )));
        let max_texture = find(&it.props, b"3dsMax|MaxTexture").map_or(&b""[..], |p| p.value_str);
        let known_shader = matches!(max_texture, b"MULTIOUTPUT_TO_OSLMap" | b"OSLMap")
            || matches!(classid, 0x896e_f2fc_44bd_743f | 0x7f9a_7b9d_6fcd_f00d);
        if known_shader {
            a.push(
                &mut out.warnings,
                a.format(format_args!(
                    "{}: OSL/select-output shader textures are not ported",
                    key(t)?
                ))?,
            )?;
        }
        let (shader, shader_dependencies) = if !known_shader
            && !layered
            && names.relative.is_empty()
            && names.absolute.is_empty()
            && video.is_none()
        {
            shader_texture(it, env, a)?
        } else {
            (None, Vec::new())
        };
        if shader.is_some() {
            out.num_shader_textures = out
                .num_shader_textures
                .checked_add(1)
                .ok_or(Error::ResourceLimit("shader textures"))?;
        }
        env.resolve(&mut names)?;
        let mut layers = Vec::new();
        if layered {
            for (s, p) in &it.dst {
                if !p.is_empty() || env.types[*s] != ElementType::Texture {
                    continue;
                }
                let Some(k) = tex_of(*s) else { continue };
                let lp = &items[textures[k]].props;
                let alpha = find_real(lp, b"Texture alpha", 1.0);
                let mode = find_enum(lp, b"BlendMode", 0, BLEND_OVERLAY);
                a.push(&mut layers, (k, mode, alpha))?;
            }
            if let Some(c) = env.child(it.node, b"Alphas") {
                if let NodeData::Reals(a) = node_data(env.doc, c, env.ascii, env.allocation)? {
                    for (l, x) in layers.iter_mut().zip(a) {
                        l.2 = x;
                    }
                }
            }
            if let Some(c) = env.child(it.node, b"BlendModes") {
                if let NodeData::Ints(m) = node_data(env.doc, c, env.ascii, env.allocation)? {
                    for (l, x) in layers.iter_mut().zip(m) {
                        if x >= 0 && i64::from(x) < BLEND_OVERLAY {
                            l.1 = i64::from(x);
                        }
                    }
                }
            }
        }
        // ufbxi_insert_texture_file.
        let mut tx = Texture {
            shader,
            shader_dependencies,
            item: t,
            layered,
            names,
            content,
            video,
            file_index: NO_INDEX,
            has_file: false,
            layers,
            file_textures: Vec::new(),
        };
        let file_key = if !tx.names.raw_absolute.is_empty() {
            Some((false, a.bytes(&tx.names.raw_absolute)?))
        } else if !tx.names.raw_relative.is_empty() {
            Some((true, a.bytes(&tx.names.raw_relative)?))
        } else {
            None
        };
        if let Some(fk) = file_key {
            let index = *file_keys.try_get_or_insert_with(fk, a, || {
                let index = files.len();
                a.push(&mut files, Names::default())?;
                a.push(&mut file_content, Vec::new())?;
                Ok(index)
            })?;
            tx.file_index = index as u64;
            tx.has_file = true;
            let (f, n) = (&mut files[index], &tx.names);
            for (dst, src) in [
                (&mut f.filename, &n.filename),
                (&mut f.relative, &n.relative),
                (&mut f.absolute, &n.absolute),
                (&mut f.raw_filename, &n.raw_filename),
                (&mut f.raw_relative, &n.raw_relative),
                (&mut f.raw_absolute, &n.raw_absolute),
                (&mut file_content[index], &tx.content),
            ] {
                if dst.is_empty() {
                    *dst = a.bytes(src)?;
                }
            }
        }
        a.push(&mut texs, tx)?;
    }

    // Second material pass: sort textures, fetch maps.
    for (m, shading_model, shader_type, prefix, mut tex, bindings) in mats {
        a.stable_sort(&mut tex, |a, b| a.material_prop.cmp(&b.material_prop))?;
        let it = &items[m];
        let (fbx_maps, pbr_maps, features) = fetch_maps(
            &key(m)?,
            shader_type,
            &it.props,
            &tex,
            &bindings,
            &mut out.warnings,
            a,
        )?;
        for table in &bindings {
            for (shader, material) in table {
                for t in tex.iter_mut().skip_while(|t| t.material_prop < *material) {
                    if t.shader_prop != *material {
                        break;
                    }
                    t.shader_prop = a.bytes(shader)?;
                }
            }
        }
        if env.lean {
            budget.push(
                &mut out.materials,
                (
                    it.element,
                    LeanMaterial {
                        features,
                        properties: {
                            let mut props = budget.try_collect(
                                it.props
                                    .authored
                                    .iter()
                                    .map(|p| (p, 0usize))
                                    .chain(it.props.defaults.iter().flatten().map(|p| (p, 1usize)))
                                    .enumerate()
                                    .map(|(ordinal, (p, layer))| {
                                        Ok((
                                            layer,
                                            ordinal,
                                            crate::SourceProperty {
                                                name: budget.bytes(p.name)?,
                                                property_type: p.kind as u8,
                                                flags: p.flags,
                                                value_int: p.value_int,
                                                value_real: p.value_real,
                                                value_str: budget.bytes(p.value_str)?,
                                                value_blob: budget.bytes(p.value_blob)?,
                                            },
                                        ))
                                    }),
                            )?;
                            props.sort_unstable_by(|a, b| {
                                (&a.2.name, a.0, a.1).cmp(&(&b.2.name, b.0, b.1))
                            });
                            props.dedup_by(|a, b| a.2.name == b.2.name);
                            budget.collect(props.into_iter().map(|(_, _, p)| p))?
                        },
                        attachments: tex,
                        shading_model,
                        fbx: fbx_maps,
                        pbr: pbr_maps,
                    },
                ),
            )?;
            continue;
        }
        let feature_obj = {
            let mut o = Obj::new();
            for (&n, (enabled, explicit)) in FEATURES.iter().zip(&features) {
                o.set(
                    n,
                    V::Obj(
                        Obj::new()
                            .with("enabled", V::Bool(*enabled))
                            .with("is_explicit", V::Bool(*explicit)),
                    ),
                );
            }
            V::Obj(o)
        };
        let obj = Obj::new()
            .with("fbx", maps_obj(&FBX_MAPS, &fbx_maps))
            .with("pbr", maps_obj(&PBR_MAPS, &pbr_maps))
            .with("features", feature_obj)
            .with("shader_type", V::I(shader_type))
            .with(
                "shader",
                V::Ref(
                    it.src
                        .iter()
                        .copied()
                        .find(|&d| env.types[d] == ElementType::Shader),
                ),
            )
            .with("shading_model_name", s(&shading_model))
            .with("shader_prop_prefix", s(&prefix))
            .with(
                "textures",
                list(tex.iter().map(|t| {
                    V::Obj(
                        Obj::new()
                            .with("material_prop", s(&t.material_prop))
                            .with("shader_prop", s(&t.shader_prop))
                            .with("texture", V::Ref(Some(t.texture))),
                    )
                })),
            );
        out.records.push((it.element, obj));
    }

    // ufbxi_fetch_file_textures (no shader textures).
    const STARTED: u8 = 1;
    const FINISHED: u8 = 2;
    let mut states = budget.filled(texs.len(), 0u8)?;
    let mut stack = budget.collect(0..texs.len())?;
    let mut marks = budget.filled(texs.len(), 0usize)?;
    let mut generation = 0usize;
    let mut dedup = |list: Vec<usize>| -> Result<Vec<usize>, crate::Error> {
        generation = generation
            .checked_add(1)
            .ok_or(crate::Error::ResourceLimit("texture traversal"))?;
        let mut seen = Vec::new();
        budget.reserve(&mut seen, list.len())?;
        for t in list {
            if marks[t] != generation {
                marks[t] = generation;
                seen.push(t);
            }
        }
        Ok(seen)
    };
    while let Some(t) = stack.pop() {
        if states[t] == FINISHED {
            continue;
        }
        if states[t] == STARTED {
            states[t] = FINISHED;
            let mut deps = Vec::new();
            if !texs[t].layered && texs[t].shader.is_none() {
                budget.push(&mut deps, t)?;
            }
            for &element in &texs[t].shader_dependencies {
                if let Some(l) = tex_of(element) {
                    if !texs[l].file_textures.is_empty() {
                        budget.push(&mut deps, l)?;
                    }
                }
            }
            for &(l, _, _) in &texs[t].layers {
                if !texs[l].file_textures.is_empty() {
                    budget.push(&mut deps, l)?;
                }
            }
            let deps = dedup(deps)?;
            texs[t].file_textures = if deps.len() == 1 {
                budget.collect(texs[deps[0]].file_textures.iter().copied())?
            } else {
                dedup(
                    budget.collect(
                        deps.iter()
                            .flat_map(|&d| texs[d].file_textures.iter().copied()),
                    )?,
                )?
            };
        } else {
            if !texs[t].layered && texs[t].shader.is_none() {
                texs[t].file_textures = budget.collect([t])?;
                states[t] = FINISHED;
                continue;
            }
            states[t] = STARTED;
            budget.push(&mut stack, t)?;
            for &element in &texs[t].shader_dependencies {
                if let Some(child) = tex_of(element) {
                    budget.push(&mut stack, child)?;
                }
            }
            for &(child, _, _) in &texs[t].layers {
                budget.push(&mut stack, child)?;
            }
        }
    }

    // Texture records with ufbxi_update_texture.
    if env.lean {
        // Videos and texture files are not part of the public scene.
        let texture_elements = budget.collect(texs.iter().map(|tx| items[tx.item].element))?;
        for tx in texs {
            let it = &items[tx.item];
            budget.push(
                &mut out.textures,
                (
                    it.element,
                    LeanTexture {
                        layers: budget.collect(
                            tx.layers
                                .iter()
                                .map(|&(t, m, a)| (texture_elements[t], m, a)),
                        )?,
                        file_textures: budget
                            .collect(tx.file_textures.iter().map(|&t| texture_elements[t]))?,
                        layered: tx.layered,
                        filename: tx.names.filename,
                        relative_filename: tx.names.relative,
                        absolute_filename: tx.names.absolute,
                        content: tx.content,
                        uv_set: budget
                            .bytes(find(&it.props, b"UVSet").map_or(&b""[..], |p| p.value_str))?,
                        wrap: [
                            find_enum(&it.props, b"WrapModeU", 0, 1),
                            find_enum(&it.props, b"WrapModeV", 0, 1),
                        ],
                    },
                ),
            )?;
        }
        return Ok(out);
    }
    for tx in &texs {
        let it = &items[tx.item];
        let mut obj = Obj::new().with(
            "type",
            V::I(if tx.shader.is_some() {
                3
            } else {
                i64::from(tx.layered)
            }),
        );
        if let Some(shader) = &tx.shader {
            obj.set("shader", V::Obj(shader.clone()));
        }
        tx.names.set(&mut obj);
        let uv = texture_transform(&it.props);
        let (has_uv, to_uv, to_tex) = if sn::is_transform_identity(&uv) {
            (false, sn::IDENTITY_MATRIX, sn::IDENTITY_MATRIX)
        } else {
            let m = sn::transform_to_matrix(&uv);
            (true, m, sn::matrix_invert(&m))
        };
        let uv_set = find(&it.props, b"UVSet").map_or(&b""[..], |p| p.value_str);
        let element = |t: usize| V::Ref(Some(items[texs[t].item].element));
        let layers = list(tx.layers.iter().map(|&(l, mode, alpha)| {
            V::Obj(
                Obj::new()
                    .with("texture", element(l))
                    .with("blend_mode", V::I(mode))
                    .with("alpha", V::F(alpha)),
            )
        }));
        let file_textures = list(tx.file_textures.iter().map(|&f| element(f)));
        obj.set("content", s(&tx.content))
            .set("video", V::Ref(tx.video))
            .set("file_index", V::U(tx.file_index))
            .set("has_file", V::Bool(tx.has_file))
            .set("layers", layers)
            .set("file_textures", file_textures)
            .set("uv_set", s(uv_set))
            .set("wrap_u", V::I(find_enum(&it.props, b"WrapModeU", 0, 1)))
            .set("wrap_v", V::I(find_enum(&it.props, b"WrapModeV", 0, 1)))
            .set("has_uv_transform", V::Bool(has_uv))
            .set(
                "uv_transform",
                V::Obj(
                    Obj::new()
                        .with("translation", V::Vec(uv.translation.to_vec()))
                        .with("rotation", V::Vec(uv.rotation.to_vec()))
                        .with("scale", V::Vec(uv.scale.to_vec())),
                ),
            )
            .set("texture_to_uv", V::Vec(to_uv.to_vec()))
            .set("uv_to_texture", V::Vec(to_tex.to_vec()));
        out.records.push((it.element, obj));
    }

    for (&v, (names, content)) in videos.iter().zip(&vids) {
        let mut obj = Obj::new().with("content", s(content));
        names.set(&mut obj);
        out.records.push((items[v].element, obj));
    }

    if !env.lean {
        // ufbxi_update_display_layer, ufbx.c:23393-23398.
        for layer in typed(ElementType::DisplayLayer)? {
            let it = &items[layer];
            let nodes = a.collect(
                it.dst
                    .iter()
                    .filter(|(s, _)| env.types[*s] == ElementType::Node)
                    .map(|(s, _)| V::Ref(Some(*s))),
            )?;
            let obj = Obj::new()
                .with("visible", V::Bool(find_int(&it.props, b"Show", 1) != 0))
                .with("frozen", V::Bool(find_int(&it.props, b"Freeze", 1) != 0))
                .with(
                    "ui_color",
                    crate::canon::v3(find_vec3(&it.props, b"Color", [f64::from(0.8f32); 3])),
                )
                .with("nodes", V::Obj(Obj::new().with("data", V::List(nodes))));
            out.records.push((it.element, obj));
        }
    }
    if !env.lean {
        for shader in typed(ElementType::Shader)? {
            let it = &items[shader];
            let bindings = a.collect(
                it.dst
                    .iter()
                    .filter(|(d, _)| env.types[*d] == ElementType::ShaderBinding)
                    .map(|(d, _)| V::Ref(Some(*d))),
            )?;
            out.records.push((
                it.element,
                Obj::new()
                    .with("type", V::I(implementation_type(&it.props)))
                    .with(
                        "bindings",
                        V::Obj(Obj::new().with("data", V::List(bindings))),
                    ),
            ));
        }
        for table in typed(ElementType::ShaderBinding)? {
            let it = &items[table];
            let entries = read_bindings(env, it.node, a)?;
            let props = a.collect(entries.iter().map(|(shader, material)| {
                V::Obj(
                    Obj::new()
                        .with("shader_prop", s(shader))
                        .with("material_prop", s(material)),
                )
            }))?;
            out.records.push((
                it.element,
                Obj::new().with(
                    "prop_bindings",
                    V::Obj(Obj::new().with("data", V::List(props))),
                ),
            ));
        }
    }

    out.texture_files = files
        .iter()
        .zip(&file_content)
        .enumerate()
        .map(|(i, (f, c))| {
            let mut o = Obj::new().with("index", V::U(i as u64));
            f.set(&mut o);
            o.set("content", s(c));
            V::Obj(o)
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_paths_resolve_like_ufbx() {
        // Relative to the DocumentUrl directory, keeping its separator.
        let mut rel = Vec::new();
        absolute_to_relative(
            &mut rel,
            b"D:\\proj\\scenes\\a.fbx",
            b"D:\\proj\\tex\\b.png",
            &Budget::new(usize::MAX),
        )
        .unwrap();
        assert_eq!(rel, b"..\\tex\\b.png");
        // Different roots leave the relative name untouched.
        let mut rel = b"keep".to_vec();
        absolute_to_relative(&mut rel, b"/x/a.fbx", b"C:/b.png", &Budget::new(usize::MAX)).unwrap();
        assert_eq!(rel, b"keep");
        // Empty relative root: leading separators dropped, separators unified.
        assert_eq!(
            resolve_relative_filename(b"\\\\tex\\b.png", &Budget::new(usize::MAX)).unwrap(),
            b"tex/b.png"
        );
        assert_eq!(
            resolve_relative_filename(b"//", &Budget::new(usize::MAX)).unwrap(),
            b""
        );
    }
}
