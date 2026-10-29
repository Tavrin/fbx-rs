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
//! leaves the affected record as far as computed): shader elements
//! (`Implementation`/`BindingTable`), shader textures
//! (`ufbxi_finalize_shader_texture`), PBR mapping tables other than the FBX
//! Lambert/Phong ones, and the 6x00-style `LayerElement*Texture` patch.
use crate::canon::{s, Obj, V};
use crate::canon_dom::{node_content, node_data, node_string, NodeData};
use crate::canon_scene::ElementType;
use crate::scene_node::{self as sn, Transform};
use crate::scene_property::{AuthoredProperty, PropertyLayers, Type};
use crate::syntax::Document;

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
const FEATURES: [&str; 23] = [
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

/// `ufbx_shader_type` values used here.
const SHADER_UNKNOWN: i64 = 0;
const SHADER_FBX_LAMBERT: i64 = 1;
const SHADER_FBX_PHONG: i64 = 2;
const SHADER_3DS_MAX_PHYSICAL_MATERIAL: i64 = 5;
const SHADER_3DS_MAX_PBR_METAL_ROUGH: i64 = 6;
const SHADER_3DS_MAX_PBR_SPEC_GLOSS: i64 = 7;
const SHADER_GLTF_MATERIAL: i64 = 8;
const SHADER_OPENPBR_MATERIAL: i64 = 9;

/// `ufbxi_shader_pbr_mappings[shader_type]` (mapping, default features) for
/// the ported shader types.
fn pbr_mapping(shader_type: i64) -> Option<(&'static [Mapping], &'static [&'static str])> {
    const PHONG_FEATURES: &[&str] = &["diffuse", "specular", "emission", "transmission"];
    const LAMBERT_FEATURES: &[&str] = &["diffuse", "emission", "transmission"];
    match shader_type {
        SHADER_UNKNOWN | SHADER_FBX_PHONG => Some((PHONG_PBR_MAPPING, PHONG_FEATURES)),
        SHADER_FBX_LAMBERT => Some((LAMBERT_PBR_MAPPING, LAMBERT_FEATURES)),
        _ => None,
    }
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
struct MaterialTexture {
    material_prop: Vec<u8>,
    shader_prop: Vec<u8>,
    texture: usize,
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
    pub children: &'a [Vec<usize>],
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
    pub(crate) shading_model: Vec<u8>,
    /// In [`FBX_MAPS`] order.
    pub(crate) fbx: Vec<Map>,
    /// In [`PBR_MAPS`] order.
    pub(crate) pbr: Vec<Map>,
}

/// The texture values the public `Scene` exposes.
pub(crate) struct LeanTexture {
    pub(crate) layered: bool,
    pub(crate) filename: Vec<u8>,
    pub(crate) relative_filename: Vec<u8>,
    pub(crate) absolute_filename: Vec<u8>,
    pub(crate) content: Vec<u8>,
    pub(crate) uv_set: Vec<u8>,
    pub(crate) wrap: [i64; 2],
}

impl Env<'_, '_> {
    fn child(&self, node: usize, name: &[u8]) -> Option<usize> {
        self.children[node]
            .iter()
            .copied()
            .find(|&c| self.doc.nodes[c].name == name)
    }
    /// `ufbxi_find_val1(node, name, "S")`: assigns only when present.
    fn val_str(&self, node: usize, name: &[u8], dst: &mut Vec<u8>) {
        if let Some(c) = self.child(node, name) {
            if let Some((Some(view), _)) = node_string(self.doc, c, self.ascii) {
                *dst = view;
            }
        }
    }
    /// `ufbxi_find_val1(node, name, "b")`: assigns only when present.
    fn val_blob(&self, node: usize, name: &[u8], dst: &mut Vec<u8>) {
        if let Some(c) = self.child(node, name) {
            if let Some((_, raw)) = node_string(self.doc, c, self.ascii) {
                *dst = raw;
            }
        }
    }
    /// The four file name reads shared by `ufbxi_read_texture` and
    /// `ufbxi_read_video`.
    fn read_names(&self, node: usize) -> Names {
        let mut n = Names::default();
        self.val_str(node, b"FileName", &mut n.absolute);
        self.val_str(node, b"Filename", &mut n.absolute);
        self.val_str(node, b"RelativeFileName", &mut n.relative);
        self.val_str(node, b"RelativeFilename", &mut n.relative);
        self.val_blob(node, b"FileName", &mut n.raw_absolute);
        self.val_blob(node, b"Filename", &mut n.raw_absolute);
        self.val_blob(node, b"RelativeFileName", &mut n.raw_relative);
        self.val_blob(node, b"RelativeFilename", &mut n.raw_relative);
        n
    }
    /// `ufbxi_resolve_filenames` for the string and the raw views.
    fn resolve(&self, n: &mut Names) {
        if n.relative.is_empty() {
            absolute_to_relative(&mut n.relative, &self.original_file_path.0, &n.absolute);
        }
        n.filename = resolve_relative_filename(&n.relative);
        if n.raw_relative.is_empty() {
            absolute_to_relative(
                &mut n.raw_relative,
                &self.original_file_path.1,
                &n.raw_absolute,
            );
        }
        n.raw_filename = resolve_relative_filename(&n.raw_relative);
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
fn absolute_to_relative(dst: &mut Vec<u8>, rel: &[u8], src: &[u8]) {
    if rel.is_empty() || src.is_empty() || rel[0] != src[0] {
        return;
    }
    let mut rel_length = rel.len();
    while rel_length > 0 && !is_separator(rel[rel_length - 1]) {
        rel_length -= 1;
    }
    if rel_length == 0 {
        return;
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
        out.extend_from_slice(&[b'.', b'.', separator]);
        rel_begin = rel_end + 1;
    }
    while src_begin < src_length {
        let src_end = next_path_segment(src, src_begin, src_length);
        out.extend_from_slice(&src[src_begin..src_end]);
        if src_end < src_length {
            out.push(separator);
        }
        src_begin = src_end + 1;
    }
    *dst = out;
}

/// `ufbxi_resolve_relative_filename` with the import profile's empty
/// `metadata.relative_root` (the scene is loaded from memory without a
/// file name, so the prefix is always empty).
fn resolve_relative_filename(src: &[u8]) -> Vec<u8> {
    let start = src.iter().position(|&c| !is_separator(c));
    let Some(start) = start else {
        return Vec::new();
    };
    src[start..]
        .iter()
        .map(|&c| if is_separator(c) { PATH_SEPARATOR } else { c })
        .collect()
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

/// `ufbxi_fetch_mapping_maps` without shader bindings (identity binding).
fn fetch_mapping_maps(
    props: &PropertyLayers<'_>,
    textures: &[MaterialTexture],
    maps: &mut [Map],
    index: fn(&str) -> usize,
    mappings: &[Mapping],
) {
    for &(target, flags, transform, name) in mappings {
        let map = &mut maps[index(target)];
        let prop = find(props, name.as_bytes());
        if let Some(p) = prop.filter(|p| p.kind != Type::Reference) {
            map.value = p.value_real;
            map.int = p.value_int;
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
        if let Some(t) = prop_texture(textures, name.as_bytes()) {
            map.texture = Some(t);
            map.texture_enabled = true;
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
fn fetch_maps(
    key: &str,
    shader_type: i64,
    props: &PropertyLayers<'_>,
    textures: &[MaterialTexture],
    warnings: &mut Vec<String>,
) -> (Vec<Map>, Vec<Map>, Vec<(bool, bool)>) {
    let mut fbx_maps = vec![Map::default(); FBX_MAPS.len()];
    let mut pbr_maps = vec![Map::default(); PBR_MAPS.len()];
    let mut features = vec![(false, false); FEATURES.len()];
    fetch_mapping_maps(props, textures, &mut fbx_maps, fbx, BASE_FBX_MAPPING);
    match pbr_mapping(shader_type) {
        Some((mapping, defaults)) => {
            for f in defaults {
                features[feature(f)].0 = true;
            }
            fetch_mapping_maps(props, textures, &mut pbr_maps, pbr, mapping);
        }
        None => warnings.push(format!(
            "{key}: PBR mapping for shader type {shader_type} is not ported"
        )),
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
    (fbx_maps, pbr_maps, features)
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

/// `ufbx_texture` state during finalization.
struct Texture {
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
pub fn project(env: &Env<'_, '_>, items: &[Item<'_>]) -> Output {
    let mut out = Output {
        records: Vec::new(),
        texture_files: Vec::new(),
        materials: Vec::new(),
        textures: Vec::new(),
        warnings: Vec::new(),
    };
    // Typed order is source order (element ordinal == DOM node + 1).
    let typed = |ty: ElementType| {
        let mut v: Vec<usize> = (0..items.len()).filter(|&i| items[i].ty == ty).collect();
        v.sort_by_key(|&i| items[i].node);
        v
    };
    let videos = typed(ElementType::Video);
    let textures = typed(ElementType::Texture);
    let materials = typed(ElementType::Material);
    let key = |i: usize| format!("element {}", items[i].element);

    // ufbxi_read_material and the first finalize pass.
    let mut mats = Vec::new();
    for &m in &materials {
        let it = &items[m];
        let mut shading_model = Vec::new();
        env.val_str(it.node, b"ShadingModel", &mut shading_model);
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
        if shader.is_some() {
            out.warnings
                .push(format!("{}: shader elements are not ported", key(m)));
        } else if shader_type == SHADER_UNKNOWN {
            let a = find_int(&it.props, b"3dsMax|ClassIDa", 0) as u64 as u32;
            let b = find_int(&it.props, b"3dsMax|ClassIDb", 0) as u64 as u32;
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
        let tex: Vec<MaterialTexture> = it
            .dst
            .iter()
            .filter(|(src, _)| env.types[*src] == ElementType::Texture)
            .map(|(src, prop)| MaterialTexture {
                material_prop: prop.clone(),
                shader_prop: prop.clone(),
                texture: *src,
            })
            .collect();
        mats.push((m, shading_model, shader_type, prefix.to_vec(), tex));
    }

    // ufbxi_resolve_file_content.
    let mut vids: Vec<(Names, Vec<u8>)> = videos
        .iter()
        .map(|&v| {
            let it = &items[v];
            let mut names = env.read_names(it.node);
            let content = env
                .child(it.node, b"Content")
                .map_or_else(Vec::new, |c| node_content(env.doc, c, env.ascii));
            env.resolve(&mut names);
            (names, content)
        })
        .collect();
    for i in 0..vids.len() {
        if !vids[i].1.is_empty() {
            continue;
        }
        let name = vids[i].0.absolute.clone();
        if let Some(j) = (0..vids.len()).find(|&j| {
            !vids[j].1.is_empty() && !vids[j].0.absolute.is_empty() && vids[j].0.absolute == name
        }) {
            vids[i].1 = vids[j].1.clone();
        }
    }

    // Texture finalize pass.
    let mut texs: Vec<Texture> = Vec::new();
    let tex_of = |element: usize| textures.iter().position(|&t| items[t].element == element);
    let mut file_keys: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut files: Vec<Names> = Vec::new();
    let mut file_content: Vec<Vec<u8>> = Vec::new();
    for &t in &textures {
        let it = &items[t];
        let layered = env.doc.nodes[it.node].name == b"LayeredTexture";
        let mut names = if layered {
            Names::default()
        } else {
            env.read_names(it.node)
        };
        let video = it
            .dst
            .iter()
            .filter(|(_, p)| p.is_empty())
            .map(|(s, _)| *s)
            .find(|&s| env.types[s] == ElementType::Video);
        let content = video
            .and_then(|v| videos.iter().position(|&x| items[x].element == v))
            .map_or_else(Vec::new, |k| vids[k].1.clone());
        // ufbxi_finalize_shader_texture: only the detection is ported.
        let classid = (find_int(&it.props, b"3dsMax|ClassIDa", 0) as u64 as u32 as u64) << 32
            | find_int(&it.props, b"3dsMax|ClassIDb", 0) as u64 as u32 as u64;
        let max_texture = find(&it.props, b"3dsMax|MaxTexture").map_or(&b""[..], |p| p.value_str);
        if max_texture == b"MULTIOUTPUT_TO_OSLMap"
            || classid == 0x896e_f2fc_44bd_743f
            || max_texture == b"OSLMap"
            || classid == 0x7f9a_7b9d_6fcd_f00d
            || (!layered
                && names.relative.is_empty()
                && names.absolute.is_empty()
                && video.is_none())
        {
            out.warnings
                .push(format!("{}: shader textures are not ported", key(t)));
        }
        env.resolve(&mut names);
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
                layers.push((k, mode, alpha));
            }
            if let Some(c) = env.child(it.node, b"Alphas") {
                if let NodeData::Reals(a) = node_data(env.doc, c, env.ascii) {
                    for (l, x) in layers.iter_mut().zip(a) {
                        l.2 = x;
                    }
                }
            }
            if let Some(c) = env.child(it.node, b"BlendModes") {
                if let NodeData::Ints(m) = node_data(env.doc, c, env.ascii) {
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
            Some((false, tx.names.raw_absolute.clone()))
        } else if !tx.names.raw_relative.is_empty() {
            Some((true, tx.names.raw_relative.clone()))
        } else {
            None
        };
        if let Some(fk) = file_key {
            let index = match file_keys.iter().position(|k| *k == fk) {
                Some(i) => i,
                None => {
                    file_keys.push(fk);
                    files.push(Names::default());
                    file_content.push(Vec::new());
                    file_keys.len() - 1
                }
            };
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
                    dst.clone_from(src);
                }
            }
        }
        texs.push(tx);
    }

    // Second material pass: sort textures, fetch maps.
    for (m, shading_model, shader_type, prefix, mut tex) in mats {
        tex.sort_by(|a, b| a.material_prop.cmp(&b.material_prop));
        let it = &items[m];
        let (fbx_maps, pbr_maps, features) =
            fetch_maps(&key(m), shader_type, &it.props, &tex, &mut out.warnings);
        if env.lean {
            out.materials.push((
                it.element,
                LeanMaterial {
                    shading_model,
                    fbx: fbx_maps,
                    pbr: pbr_maps,
                },
            ));
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
    let mut states = vec![0u8; texs.len()];
    let mut stack: Vec<usize> = (0..texs.len()).collect();
    let dedup = |list: Vec<usize>| {
        let mut seen = Vec::new();
        for t in list {
            if !seen.contains(&t) {
                seen.push(t);
            }
        }
        seen
    };
    while let Some(t) = stack.pop() {
        if states[t] == FINISHED {
            continue;
        }
        if states[t] == STARTED {
            states[t] = FINISHED;
            let mut deps = Vec::new();
            if !texs[t].layered {
                deps.push(t);
            }
            for &(l, _, _) in &texs[t].layers {
                if !texs[l].file_textures.is_empty() {
                    deps.push(l);
                }
            }
            let deps = dedup(deps);
            texs[t].file_textures = if deps.len() == 1 {
                texs[deps[0]].file_textures.clone()
            } else {
                dedup(
                    deps.iter()
                        .flat_map(|&d| texs[d].file_textures.clone())
                        .collect(),
                )
            };
        } else {
            if !texs[t].layered {
                texs[t].file_textures = vec![t];
                states[t] = FINISHED;
                continue;
            }
            states[t] = STARTED;
            stack.push(t);
            stack.extend(texs[t].layers.iter().map(|l| l.0));
        }
    }

    // Texture records with ufbxi_update_texture.
    if env.lean {
        // Videos and texture files are not part of the public scene.
        for tx in texs {
            let it = &items[tx.item];
            out.textures.push((
                it.element,
                LeanTexture {
                    layered: tx.layered,
                    filename: tx.names.filename,
                    relative_filename: tx.names.relative,
                    absolute_filename: tx.names.absolute,
                    content: tx.content,
                    uv_set: find(&it.props, b"UVSet").map_or(Vec::new(), |p| p.value_str.to_vec()),
                    wrap: [
                        find_enum(&it.props, b"WrapModeU", 0, 1),
                        find_enum(&it.props, b"WrapModeV", 0, 1),
                    ],
                },
            ));
        }
        return out;
    }
    for tx in &texs {
        let it = &items[tx.item];
        let mut obj = Obj::new().with("type", V::I(i64::from(tx.layered)));
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
    out
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
        );
        assert_eq!(rel, b"..\\tex\\b.png");
        // Different roots leave the relative name untouched.
        let mut rel = b"keep".to_vec();
        absolute_to_relative(&mut rel, b"/x/a.fbx", b"C:/b.png");
        assert_eq!(rel, b"keep");
        // Empty relative root: leading separators dropped, separators unified.
        assert_eq!(resolve_relative_filename(b"\\\\tex\\b.png"), b"tex/b.png");
        assert_eq!(resolve_relative_filename(b"//"), b"");
    }
}
