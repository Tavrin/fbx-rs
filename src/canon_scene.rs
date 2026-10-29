//! Rust scene to `fbx-scene-canon/1` (engine-converted profile) projection.
//!
//! Element identities, ordering, domains, connections and the element header
//! follow reference/canon.c; element semantics follow ufbx 0.23.1 (MIT,
//! <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>, `ufbxi_read_object`,
//! `ufbxi_read_property(ies)`). Fields this projection does not yet derive are
//! emitted as zero values and show up as differences in the corpus comparison
//! (docs/VERIFICATION.md describes the comparison).
use crate::canon::{s, Obj, V};
use crate::scene::{self, ConnectionKind};
use crate::scene_property::{self, AuthoredProperty};
use crate::syntax::{self, Document, Property};
#[cfg(feature = "parity")]
use crate::{canon::Emitter, canonical_writer::CanonicalWriter};
#[cfg(feature = "parity")]
use serde_json::json;
#[cfg(feature = "parity")]
use sha2::{Digest, Sha256};
#[cfg(feature = "parity")]
use std::io::{self, Write};

/// Limits for one canonical dump (on top of the syntax and index limits).
#[derive(Debug, Clone)]
pub struct DumpLimits {
    pub syntax: syntax::Limits,
    pub index: scene::Limits,
    /// Mesh/triangulation draft limits used for the triangles section.
    #[cfg(feature = "parity")]
    pub draft: crate::scene_draft::Limits,
    pub max_properties: usize,
    pub max_keys: usize,
    pub max_allocation: usize,
    /// Bound on deformer traversal, lookup and sorting work.
    pub max_deformer_work: usize,
    pub max_line: usize,
    pub max_output: usize,
}
impl Default for DumpLimits {
    fn default() -> Self {
        Self {
            syntax: syntax::Limits::default(),
            index: scene::Limits::default(),
            #[cfg(feature = "parity")]
            draft: crate::scene_draft::Limits::default(),
            max_keys: 1 << 24,
            max_properties: 1 << 20,
            max_allocation: 512 << 20,
            max_deformer_work: 1 << 28,
            max_line: 64 << 20,
            // 8 GiB, saturated to the address space on 32-bit targets (wasm32).
            max_output: usize::try_from(8u64 << 30).unwrap_or(usize::MAX),
        }
    }
}

/// `ufbx_element_type` discriminants (ufbx.h 0.23.1 declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementType {
    Unknown = 0,
    Node = 1,
    Mesh = 2,
    Light = 3,
    Camera = 4,
    Bone = 5,
    Empty = 6,
    LineCurve = 7,
    NurbsCurve = 8,
    NurbsSurface = 9,
    NurbsTrimSurface = 10,
    NurbsTrimBoundary = 11,
    StereoCamera = 13,
    CameraSwitcher = 14,
    Marker = 15,
    LodGroup = 16,
    SkinDeformer = 17,
    SkinCluster = 18,
    BlendDeformer = 19,
    BlendChannel = 20,
    BlendShape = 21,
    CacheDeformer = 22,
    CacheFile = 23,
    Material = 24,
    Texture = 25,
    Video = 26,
    Shader = 27,
    ShaderBinding = 28,
    AnimStack = 29,
    AnimLayer = 30,
    AnimValue = 31,
    AnimCurve = 32,
    DisplayLayer = 33,
    SelectionSet = 34,
    SelectionNode = 35,
    Character = 36,
    Constraint = 37,
    AudioLayer = 38,
    AudioClip = 39,
    Pose = 40,
    MetadataObject = 41,
}
impl ElementType {
    fn schema(self) -> &'static str {
        use ElementType::*;
        match self {
            Unknown => "ufbx_unknown",
            Node => "ufbx_node",
            Mesh => "ufbx_mesh",
            Light => "ufbx_light",
            Camera => "ufbx_camera",
            Bone => "ufbx_bone",
            Empty => "ufbx_empty",
            LineCurve => "ufbx_line_curve",
            NurbsCurve => "ufbx_nurbs_curve",
            NurbsSurface => "ufbx_nurbs_surface",
            NurbsTrimSurface => "ufbx_nurbs_trim_surface",
            NurbsTrimBoundary => "ufbx_nurbs_trim_boundary",
            StereoCamera => "ufbx_stereo_camera",
            CameraSwitcher => "ufbx_camera_switcher",
            Marker => "ufbx_marker",
            LodGroup => "ufbx_lod_group",
            SkinDeformer => "ufbx_skin_deformer",
            SkinCluster => "ufbx_skin_cluster",
            BlendDeformer => "ufbx_blend_deformer",
            BlendChannel => "ufbx_blend_channel",
            BlendShape => "ufbx_blend_shape",
            CacheDeformer => "ufbx_cache_deformer",
            CacheFile => "ufbx_cache_file",
            Material => "ufbx_material",
            Texture => "ufbx_texture",
            Video => "ufbx_video",
            Shader => "ufbx_shader",
            ShaderBinding => "ufbx_shader_binding",
            AnimStack => "ufbx_anim_stack",
            AnimLayer => "ufbx_anim_layer",
            AnimValue => "ufbx_anim_value",
            AnimCurve => "ufbx_anim_curve",
            DisplayLayer => "ufbx_display_layer",
            SelectionSet => "ufbx_selection_set",
            SelectionNode => "ufbx_selection_node",
            Character => "ufbx_character",
            Constraint => "ufbx_constraint",
            AudioLayer => "ufbx_audio_layer",
            AudioClip => "ufbx_audio_clip",
            Pose => "ufbx_pose",
            MetadataObject => "ufbx_metadata_object",
        }
    }
    /// canon.c `domain()`.
    pub fn domain(self) -> &'static str {
        use ElementType::*;
        match self {
            Node | Bone | Empty => "graph",
            Mesh => "meshes",
            SkinDeformer | SkinCluster | Pose => "skins",
            BlendDeformer | BlendChannel | BlendShape => "blend_shapes",
            Material | Texture | Video | Shader | ShaderBinding => "materials",
            Light | Camera => "cameras_lights",
            AnimStack | AnimLayer | AnimValue | AnimCurve => "curves",
            _ => "advanced",
        }
    }
    /// Compatibility query for the former animation deferral. Animation
    /// records are now supported, so no element requires that deferral.
    pub fn needs_later_phase(self) -> bool {
        false
    }
}

/// ufbx object dispatch (`ufbxi_read_object`) for 7.x objects.
fn element_type(name: &[u8], sub: &[u8]) -> Option<ElementType> {
    use ElementType::*;
    Some(match name {
        b"Model" => Node,
        b"NodeAttribute" => match sub {
            b"Light" => Light,
            b"Camera" => Camera,
            b"LimbNode" | b"Limb" | b"Root" => Bone,
            b"Null" | b"Marker" => Empty,
            b"CameraStereo" => StereoCamera,
            b"CameraSwitcher" => CameraSwitcher,
            b"FKEffector" | b"IKEffector" => Marker,
            b"LodGroup" => LodGroup,
            _ => Unknown,
        },
        b"Geometry" => match sub {
            b"Mesh" => Mesh,
            b"Shape" => BlendShape,
            b"NurbsCurve" => NurbsCurve,
            b"NurbsSurface" => NurbsSurface,
            b"Line" => LineCurve,
            b"TrimNurbsSurface" => NurbsTrimSurface,
            b"Boundary" => NurbsTrimBoundary,
            _ => Unknown,
        },
        b"Deformer" => match sub {
            b"Skin" => SkinDeformer,
            b"Cluster" => SkinCluster,
            b"BlendShape" => BlendDeformer,
            b"BlendShapeChannel" => BlendChannel,
            b"VertexCacheDeformer" => CacheDeformer,
            _ => Unknown,
        },
        b"Material" => Material,
        b"Texture" | b"LayeredTexture" => Texture,
        b"Video" => Video,
        b"AnimationStack" => AnimStack,
        b"AnimationLayer" => AnimLayer,
        b"AnimationCurveNode" => AnimValue,
        b"AnimationCurve" => AnimCurve,
        b"Pose" => Pose,
        b"Implementation" => Shader,
        b"BindingTable" => ShaderBinding,
        b"Collection" if sub == b"SelectionSet" => SelectionSet,
        b"Collection" => return None,
        b"CollectionExclusive" if sub == b"DisplayLayer" => DisplayLayer,
        b"CollectionExclusive" => return None,
        b"SelectionNode" => SelectionNode,
        b"Constraint" if sub == b"Character" => Character,
        b"Constraint" => Constraint,
        b"SceneInfo" | b"GlobalSettings" => return None,
        b"Cache" => CacheFile,
        b"ObjectMetaData" => MetadataObject,
        b"AudioLayer" => AudioLayer,
        b"Audio" => AudioClip,
        _ => Unknown,
    })
}

/// One canonical element: identity, typed projection and source binding.
#[derive(Debug)]
pub struct Element {
    pub key: String,
    pub ordinal: usize,
    pub source_id: Option<i64>,
    pub ty: ElementType,
    /// DOM node of the source object (None for synthetic elements).
    pub node: Option<usize>,
    /// Object name (`ufbxi_split_type_and_name`); empty for the root.
    pub name: Vec<u8>,
    /// `connections_src` in file order: (destination element, dst prop).
    pub(crate) src_conns: Vec<(usize, Vec<u8>)>,
    /// `connections_dst` in file order: (source element, dst prop).
    pub(crate) dst_conns: Vec<(usize, Vec<u8>)>,
    /// `ufbx_element.instances`: nodes using this attribute.
    pub(crate) instances: Vec<usize>,
    /// Canonical record (empty in a lean projection).
    pub obj: Obj,
}

/// Typed results of a lean projection (public `load`), by element index.
#[derive(Default)]
pub(crate) struct Lean {
    /// Effective parent (top-level nodes hang from the root 0).
    pub(crate) parent: Vec<Option<usize>>,
    pub(crate) children: Vec<Vec<usize>>,
    /// Material elements connected to each node, in connection order.
    pub(crate) node_materials: Vec<Vec<usize>>,
    /// Attribute elements of each node, in connection order.
    pub(crate) attribs: Vec<Vec<usize>>,
    pub(crate) visible: Vec<bool>,
    /// Converted meshes with their material elements.
    pub(crate) meshes: Vec<(usize, crate::scene_geometry::Mesh, Vec<usize>)>,
    pub(crate) materials: Vec<(usize, crate::scene_material::LeanMaterial)>,
    pub(crate) textures: Vec<(usize, crate::scene_material::LeanTexture)>,
    pub(crate) cameras: Vec<(usize, crate::scene_attribute::Camera)>,
    pub(crate) lights: Vec<(usize, crate::scene_attribute::Light)>,
}

/// Summary of one dump for corpus accounting.
#[derive(Debug, Default)]
pub struct DumpSummary {
    pub elements: usize,
    /// Compatibility list of deferred animation types (now empty).
    pub later_phase: Vec<(ElementType, usize)>,
    /// [`CanonScene::unsupported`] of the dumped scene.
    pub unsupported: Vec<(scene::UnsupportedKind, i64)>,
    /// [`CanonScene::warnings`] of the dumped scene.
    pub warnings: Vec<String>,
}

fn props_of<'a>(doc: &'a Document<'a>, i: usize) -> &'a [Property<'a>] {
    doc.properties
        .get(doc.nodes[i].properties.clone())
        .unwrap_or(&[])
}
fn string_of<'a>(p: Option<&'a Property<'a>>) -> &'a [u8] {
    match p {
        Some(Property::String(v)) => v.as_ref(),
        Some(Property::AsciiBare(v)) => v,
        _ => b"",
    }
}
/// `ufbxi_split_type_and_name`: binary `Name\0\x01Class`, ASCII `Class::Name`.
fn object_name(raw: &[u8]) -> &[u8] {
    if let Some(p) = raw.windows(2).position(|w| w == b"\x00\x01") {
        return &raw[..p];
    }
    if let Some(p) = raw.windows(2).position(|w| w == b"::") {
        return &raw[p + 2..];
    }
    raw
}

fn prop_obj(p: &AuthoredProperty<'_>) -> V {
    V::Obj(
        Obj::new()
            .with("name", s(p.name))
            .with("type", V::I(p.kind as i64))
            .with("flags", V::I(i64::from(p.flags)))
            .with("value_str", s(p.value_str))
            .with("value_blob", s(p.value_blob))
            .with("value_int", V::I(p.value_int))
            .with("value_real_arr", V::Vec(p.value_real.to_vec())),
    )
}
fn props_value(layers: &scene_property::PropertyLayers<'_>) -> V {
    let mut props = Obj::new().with(
        "props",
        V::Obj(Obj::new().with(
            "data",
            V::List(layers.authored.iter().map(prop_obj).collect()),
        )),
    );
    if let Some(defaults) = &layers.defaults {
        props.set(
            "defaults",
            V::Obj(Obj::new().with(
                "props",
                V::Obj(Obj::new().with("data", V::List(defaults.iter().map(prop_obj).collect()))),
            )),
        );
    }
    V::Obj(props)
}

// Synthesized DeformPercent replaces the authored list but preserves templates.
fn percent_props(layers: &scene_property::PropertyLayers<'_>, percent: f64, connected: bool) -> V {
    let prop = Obj::new()
        .with("name", s(b"DeformPercent"))
        .with("type", V::I(3))
        .with("flags", V::I(if connected { 0x8000 } else { 0 }))
        .with("value_real_arr", V::Vec(vec![percent, 0.0, 0.0, 0.0]));
    let mut value = props_value(layers);
    if let V::Obj(o) = &mut value {
        o.set(
            "props",
            V::Obj(Obj::new().with("data", V::List(vec![V::Obj(prop)]))),
        );
    }
    value
}

/// The projected scene: elements in canonical order plus scene-level records.
pub struct CanonScene<'a> {
    pub doc: Document<'a>,
    pub elements: Vec<Element>,
    pub connections: Vec<V>,
    pub settings: Obj,
    pub metadata: Obj,
    pub ascii: bool,
    /// Loaded node transforms by element index (`None` for non-nodes).
    pub node_states: Vec<Option<crate::scene_node::NodeState>>,
    /// `geometry_to_node` that `ufbxi_modify_geometry` bakes into attributes,
    /// by element index (`Some` only for nodes with a geometry transform).
    pub geometry_modify: Vec<Option<crate::scene_node::Mat>>,
    /// Evaluated takes: (key, time_begin, time_end). One `default` take with
    /// zero bounds when the file has no animation stacks.
    pub takes: Vec<(String, f64, f64)>,
    /// `metadata.original_file_path` (`DocumentUrl`): UTF-8 view, raw bytes.
    pub original_file_path: (Vec<u8>, Vec<u8>),
    /// `scene.texture_files` records (see `scene_material`).
    pub texture_files: Vec<V>,
    /// `uc->exporter` (`ufbx_exporter` value) after `ufbxi_match_exporter`:
    /// selects exporter quirks such as `metadata.ortho_size_unit`.
    pub exporter: i64,
    /// Content this converter does not convert (geometric curves and
    /// surfaces, caches, constraints, audio),
    /// with the source object ID (the owning `Geometry` for
    /// [`scene::UnsupportedKind::LegacyShape`]): objects in DOM order, then
    /// legacy in-geometry shapes in DOM order.
    pub unsupported: Vec<(scene::UnsupportedKind, i64)>,
    /// `scene.anim`: the first animation stack's `ufbx_anim` (empty without
    /// stacks).
    pub anim: Obj,
    pub(crate) animation: crate::animation::Animation,
    pub(crate) scale_helpers: Vec<(usize, usize)>,
    /// Lean projection (public `load`): no per-element canonical records are
    /// built; the converted values are kept typed in [`CanonScene::typed`].
    pub(crate) lean: bool,
    /// Typed results (lean projection only).
    pub(crate) typed: Lean,
    pub(crate) deformers: crate::scene_deform::Deformers,
    /// Conversion warnings: content that is read but not ported, meshes
    /// that failed to convert.
    pub warnings: Vec<String>,
}

/// `ufbxi_update_anim_stack`: take bounds from LocalStart/LocalStop, else
/// ReferenceStart/ReferenceStop, in KTime units of 46186158000 per second
/// (the FBX 7 unit; `metadata.ktime_second` uses the same constant).
fn project_takes(scene: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    const KTIME: f64 = 46_186_158_000.0;
    let mut takes = Vec::new();
    for e in scene.elements.iter_mut() {
        let (ElementType::AnimStack, Some(d)) = (e.ty, e.node) else {
            continue;
        };
        let props =
            scene_property::layers(&scene.doc, d, limits.max_properties, limits.max_allocation)
                .map_err(|err| format!("stack properties: {err:?}"))?;
        let (mut begin, mut end) = (props.find(b"LocalStart"), props.find(b"LocalStop"));
        if begin.is_none() || end.is_none() {
            begin = props.find(b"ReferenceStart");
            end = props.find(b"ReferenceStop");
        }
        let (mut t0, mut t1) = (0.0, 0.0);
        if let (Some(b), Some(x)) = (begin, end) {
            t0 = b.value_int as f64 / KTIME;
            t1 = x.value_int as f64 / KTIME;
        }
        e.obj.set("time_begin", V::F(t0));
        e.obj.set("time_end", V::F(t1));
        takes.push((e.key.clone(), t0, t1));
    }
    if takes.is_empty() {
        takes.push(("default".to_string(), 0.0, 0.0));
    }
    scene.takes = takes;
    project_anim_structure(scene, limits)
}

/// Animation stack and layer structure from `ufbxi_finalize_scene`: stack
/// `layers` (`ufbxi_fetch_dst_elements(..., UFBX_ELEMENT_ANIM_LAYER)`), the
/// `ufbx_anim` of every stack and layer (`ufbxi_push_anim`, bounds from
/// `ufbxi_update_anim_stack`), layer blend mode, weight and accumulation
/// modes, layer `anim_values`, and `scene.anim` (`ufbxi_update_anim`: the
/// first stack in element order). The animation projection stage subsequently
/// adds property lists (`anim_props`), curves and evaluation data.
fn project_anim_structure(scene: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    use ElementType::*;
    let anim = |t0: f64, t1: f64, layers: &[usize]| {
        V::Obj(
            Obj::new()
                .with("time_begin", V::F(t0))
                .with("time_end", V::F(t1))
                .with(
                    "layers",
                    V::List(layers.iter().map(|&k| V::Ref(Some(k))).collect()),
                ),
        )
    };
    // `ufbxi_fetch_dst_elements(..., ignore_duplicates = true, ...)`.
    let sources_of = |scene: &CanonScene<'_>, i: usize, ty: ElementType| {
        let mut out: Vec<usize> = Vec::new();
        for (src, prop) in dst_connections(&scene.elements[i]) {
            if prop.is_empty() && scene.elements[src].ty == ty && !out.contains(&src) {
                out.push(src);
            }
        }
        out
    };
    let mut first_stack: Option<(usize, usize)> = None;
    for i in 0..scene.elements.len() {
        let (ty, node) = (scene.elements[i].ty, scene.elements[i].node);
        match (ty, node) {
            (AnimStack, Some(node)) => {
                let layers = sources_of(scene, i, AnimLayer);
                let f = |name| match scene.elements[i].obj.get(name) {
                    Some(V::F(x)) => *x,
                    _ => 0.0,
                };
                let a = anim(f("time_begin"), f("time_end"), &layers);
                let obj = &mut scene.elements[i].obj;
                obj.set(
                    "layers",
                    V::List(layers.iter().map(|&k| V::Ref(Some(k))).collect()),
                );
                obj.set("anim", a);
                if first_stack.is_none_or(|(n, _)| node < n) {
                    first_stack = Some((node, i));
                }
            }
            (AnimLayer, Some(node)) => {
                let values = sources_of(scene, i, AnimValue);
                let props = scene_property::layers(
                    &scene.doc,
                    node,
                    limits.max_properties,
                    limits.max_allocation,
                )
                .map_err(|err| format!("layer properties: {err:?}"))?;
                let int = |name: &[u8]| props.find(name).map_or(0, |p| p.value_int);
                let (blended, additive) = match int(b"BlendMode") {
                    0 => (true, true),
                    2 => (true, false),
                    _ => (false, false),
                };
                // The ANIMATED flag marks a property an anim value drives.
                let weight_animated = dst_connections(&scene.elements[i])
                    .iter()
                    .any(|(src, prop)| prop == b"Weight" && scene.elements[*src].ty == AnimValue);
                let (weight, weight_is_animated) = match props.find(b"Weight") {
                    Some(p) => {
                        let mut w = p.value_real[0] / 100.0;
                        if w < 0.0 {
                            w = 0.0;
                        }
                        if w > f64::from(0.99999f32) {
                            w = 1.0;
                        }
                        (w, weight_animated)
                    }
                    None => (1.0, false),
                };
                let compose_rotation = int(b"RotationAccumulationMode") == 0;
                let compose_scale = int(b"ScaleAccumulationMode") == 0;
                let a = anim(0.0, 0.0, &[i]);
                let obj = &mut scene.elements[i].obj;
                obj.set("weight", V::F(weight));
                obj.set("weight_is_animated", V::Bool(weight_is_animated));
                obj.set("blended", V::Bool(blended));
                obj.set("additive", V::Bool(additive));
                obj.set("compose_rotation", V::Bool(compose_rotation));
                obj.set("compose_scale", V::Bool(compose_scale));
                obj.set(
                    "anim_values",
                    V::List(values.iter().map(|&k| V::Ref(Some(k))).collect()),
                );
                obj.set("anim", a);
            }
            _ => {}
        }
    }
    if let Some((_, i)) = first_stack {
        if let Some(V::Obj(a)) = scene.elements[i].obj.get("anim") {
            scene.anim = a.clone();
        }
    }
    Ok(())
}

/// Build the canonical projection of `input`.
pub fn project<'a>(input: &'a [u8], limits: &DumpLimits) -> Result<CanonScene<'a>, String> {
    project_with(input, limits, false)
}

/// [`project`], or the lean variant used by the public `load` (see
/// [`CanonScene::lean`]).
pub(crate) fn project_with<'a>(
    input: &'a [u8],
    limits: &DumpLimits,
    lean: bool,
) -> Result<CanonScene<'a>, String> {
    let doc = syntax::parse(input, &limits.syntax).map_err(|e| format!("syntax: {e:?}"))?;
    if !syntax::supported_version(doc.version) {
        return Err(format!("syntax: UnsupportedVersion {}", doc.version));
    }
    let ascii = !input.starts_with(b"Kaydara FBX Binary  \x00\x1a");
    let (mut elements, mut connections, unsupported) = {
        let index = scene::index(&doc, &limits.index).map_err(|e| format!("index: {e:?}"))?;
        let mut elements = vec![Element {
            key: "root".into(),
            ordinal: usize::MAX,
            source_id: None,
            ty: ElementType::Node,
            node: None,
            name: Vec::new(),
            src_conns: Vec::new(),
            dst_conns: Vec::new(),
            instances: Vec::new(),
            obj: Obj::new(),
        }];
        // Object index -> element index.
        let mut by_object = vec![None; index.objects.len()];
        for (oi, o) in index.objects.iter().enumerate() {
            let p = props_of(&doc, o.node);
            let ty = match element_type(doc.nodes[o.node].name, string_of(p.get(2))) {
                Some(t) => t,
                None => continue,
            };
            let ordinal = o.node + 1;
            by_object[oi] = Some(elements.len());
            elements.push(Element {
                key: format!("id:{}:ordinal:{ordinal}", o.id),
                ordinal,
                source_id: Some(o.id),
                ty,
                node: Some(o.node),
                name: object_name(string_of(p.get(1))).to_vec(),
                src_conns: Vec::new(),
                dst_conns: Vec::new(),
                instances: Vec::new(),
                obj: Obj::new(),
            });
        }
        let mut connections = Vec::new();
        for c in &index.connections {
            let (Some(src), dst) = (by_object[c.source], c.destination.map(|d| by_object[d]))
            else {
                continue;
            };
            let dst = match dst {
                None => 0,
                Some(Some(d)) => d,
                Some(None) => continue,
            };
            let dst_prop: &[u8] = match c.kind {
                ConnectionKind::Object => b"",
                ConnectionKind::Property(p) => p,
            };
            connections.push((src, dst, dst_prop.to_vec()));
        }
        let unsupported: Vec<(scene::UnsupportedKind, i64)> = index
            .unsupported
            .iter()
            .map(|u| (u.kind, index.objects[u.object].id))
            .collect();
        let unsupported = unsupported
            .into_iter()
            .filter(|(kind, _)| {
                !matches!(
                    kind,
                    scene::UnsupportedKind::Skin
                        | scene::UnsupportedKind::SkinCluster
                        | scene::UnsupportedKind::BlendShape
                        | scene::UnsupportedKind::BlendShapeChannel
                        | scene::UnsupportedKind::ShapeGeometry
                        | scene::UnsupportedKind::LegacyShape
                        | scene::UnsupportedKind::Pose
                )
            })
            .collect();
        (elements, connections, unsupported)
    };
    // FBX 7.1 stores shapes inside the base Geometry. ufbx creates one
    // deformer and one channel/target pair per nested Shape.
    // Index nested shapes once; static meshes need no per-mesh DOM scan.
    let mut legacy_shapes = std::collections::HashMap::<usize, Vec<usize>>::new();
    for (i, n) in doc.nodes.iter().enumerate().take(if doc.version <= 7100 {
        doc.nodes.len()
    } else {
        0
    }) {
        if let Some(parent) = n.parent.filter(|_| n.name == b"Shape") {
            legacy_shapes.entry(parent).or_default().push(i);
        }
    }
    let bases: Vec<(usize, usize)> = elements
        .iter()
        .enumerate()
        .take(if legacy_shapes.is_empty() {
            0
        } else {
            elements.len()
        })
        .filter_map(|(i, e)| {
            (e.ty == ElementType::Mesh)
                .then_some(e.node.map(|n| (i, n)))
                .flatten()
        })
        .collect();
    let mut synthetic_counts = [0usize; 3];
    let mut property_links = Vec::new();
    for (base, node) in bases {
        let Some(shapes) = legacy_shapes.remove(&node) else {
            continue;
        };
        let mut deformer = None;
        for shape in shapes {
            let name = string_of(props_of(&doc, shape).first()).to_vec();
            if deformer.is_none() {
                let k = elements.len();
                deformer = Some(k);
                elements.push(Element {
                    key: format!("id:0:ordinal:{}", shape + 1),
                    ordinal: shape + 1,
                    source_id: Some(0),
                    ty: ElementType::BlendDeformer,
                    node: Some(shape),
                    name: name.clone(),
                    src_conns: Vec::new(),
                    dst_conns: Vec::new(),
                    instances: Vec::new(),
                    obj: Obj::new(),
                });
                connections.push((k, base, Vec::new()));
            }
            let channel = elements.len();
            for (offset, ty) in [ElementType::BlendChannel, ElementType::BlendShape]
                .into_iter()
                .enumerate()
            {
                let shared = deformer.is_some_and(|k| elements[k].node == Some(shape));
                let key = if offset == 0 && !shared {
                    format!("id:0:ordinal:{}", shape + 1)
                } else {
                    let count = &mut synthetic_counts[offset + 1];
                    let key = format!(
                        "synthetic:root:role:scene-generated:type:{}:ordinal:{}",
                        ty as i64, *count
                    );
                    *count += 1;
                    key
                };
                elements.push(Element {
                    key,
                    ordinal: shape + 1,
                    source_id: Some(0),
                    ty,
                    node: Some(shape),
                    name: name.clone(),
                    src_conns: Vec::new(),
                    dst_conns: Vec::new(),
                    instances: Vec::new(),
                    obj: Obj::new(),
                });
            }
            let owner_props =
                scene_property::layers(&doc, node, limits.max_properties, limits.max_allocation)
                    .map_err(|e| format!("shape owner properties: {e:?}"))?;
            if owner_props.find(&name).is_some_and(|p| {
                matches!(
                    p.kind,
                    scene_property::Type::Number | scene_property::Type::Integer
                )
            }) {
                property_links.push((base, channel, name.clone()));
            }
            connections.push((channel, deformer.unwrap(), Vec::new()));
            connections.push((channel + 1, channel, Vec::new()));
        }
    }
    let scale_helpers = crate::scene_animation::scale_helpers(
        &doc,
        &mut elements,
        &mut connections,
        ascii,
        limits,
    )?;
    let mut scene = CanonScene {
        doc,
        elements,
        connections: Vec::new(),
        settings: Obj::new(),
        metadata: Obj::new(),
        ascii,
        node_states: Vec::new(),
        geometry_modify: Vec::new(),
        takes: Vec::new(),
        original_file_path: (Vec::new(), Vec::new()),
        texture_files: Vec::new(),
        exporter: EXPORTER_UNKNOWN,
        unsupported,
        anim: Obj::new(),
        animation: Default::default(),
        scale_helpers,
        lean,
        typed: Lean::default(),
        deformers: Default::default(),
        warnings: Vec::new(),
    };
    // Canonical element order: signed source ID then DOM ordinal; root first.
    let mut order: Vec<usize> = (0..scene.elements.len()).collect();
    order.sort_by(|&a, &b| {
        let a = &scene.elements[a];
        let b = &scene.elements[b];
        a.source_id
            .unwrap_or(i64::MIN)
            .cmp(&b.source_id.unwrap_or(i64::MIN))
            .then(a.ordinal.cmp(&b.ordinal))
            .then(a.key.cmp(&b.key))
    });
    let mut remap = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let mut taken: Vec<Option<Element>> = scene.elements.drain(..).map(Some).collect();
    scene.elements = order.iter().filter_map(|&old| taken[old].take()).collect();
    scene.scale_helpers = scene
        .scale_helpers
        .iter()
        .map(|&(o, h)| (remap[o], remap[h]))
        .collect();
    let conn = |src: usize, dst: usize, prop: &[u8]| {
        V::Obj(
            Obj::new()
                .with("src", V::Ref(Some(src)))
                .with("dst", V::Ref(Some(dst)))
                .with("src_prop", s(b""))
                .with("dst_prop", s(prop)),
        )
    };
    let mut src_lists = vec![Vec::new(); scene.elements.len()];
    let mut dst_lists = vec![Vec::new(); scene.elements.len()];
    let mut oo = Vec::new();
    for (src, dst, prop) in connections {
        let (src, dst) = (remap[src], remap[dst]);
        if prop.is_empty() {
            oo.push((src, dst));
        }
        if !lean {
            let c = conn(src, dst, &prop);
            src_lists[src].push(c.clone());
            dst_lists[dst].push(c.clone());
            scene.connections.push(c);
        }
        scene.elements[src].src_conns.push((dst, prop.clone()));
        scene.elements[dst].dst_conns.push((src, prop));
    }
    for (src, dst, name) in property_links {
        let (src, dst) = (remap[src], remap[dst]);
        let prop = b"DeformPercent".to_vec();
        if !lean {
            let mut c = conn(src, dst, &prop);
            if let V::Obj(o) = &mut c {
                o.set("src_prop", s(&name));
            }
            src_lists[src].push(c.clone());
            dst_lists[dst].push(c.clone());
            scene.connections.push(c);
        }
        scene.elements[src].src_conns.push((dst, prop.clone()));
        scene.elements[dst].dst_conns.push((src, prop));
    }
    if lean {
        return project_rest(scene, input, limits, &oo);
    }
    for (i, e) in scene.elements.iter_mut().enumerate() {
        let mut header = Obj::new()
            .with("type", V::I(e.ty as i64))
            .with(
                "connections_src",
                V::List(std::mem::take(&mut src_lists[i])),
            )
            .with(
                "connections_dst",
                V::List(std::mem::take(&mut dst_lists[i])),
            );
        if let Some(n) = e.node {
            header.set("name", s(&e.name));
            header.set("dom_node", V::Dom(Some(n + 1)));
            let layers = if scene.doc.nodes[n].name == b"Shape" {
                scene_property::PropertyLayers {
                    authored: Vec::new(),
                    defaults: None,
                }
            } else {
                crate::scene_animation::node_layers(
                    &scene.doc,
                    n,
                    scene.scale_helpers.iter().any(|&(_, h)| h == i),
                    scene.scale_helpers.iter().any(|&(o, _)| o == i),
                    limits,
                )?
            };
            if scene.doc.nodes[n].name == b"Shape" && e.ty == ElementType::BlendChannel {
                let owner = scene.doc.nodes[n].parent.ok_or("shape: missing geometry")?;
                let parent_props = scene_property::layers(
                    &scene.doc,
                    owner,
                    limits.max_properties,
                    limits.max_allocation,
                )
                .map_err(|e| format!("shape properties: {e:?}"))?;
                let source = parent_props.find(&e.name).filter(|p| {
                    matches!(
                        p.kind,
                        scene_property::Type::Number | scene_property::Type::Integer
                    )
                });
                let percent = source.map_or(0.0, |p| p.value_real[0]);
                header.set("props", percent_props(&layers, percent, source.is_some()));
            } else if e.ty == ElementType::BlendChannel && layers.authored.is_empty() {
                let direct = scene
                    .doc
                    .nodes
                    .iter()
                    .enumerate()
                    .find(|(_, d)| d.parent == Some(n) && d.name == b"DeformPercent");
                if let Some((d, _)) = direct {
                    let percent = match crate::canon_dom::node_data(&scene.doc, d, scene.ascii) {
                        crate::canon_dom::NodeData::Values(v) => match v.first() {
                            Some(crate::canon_dom::Scalar::Num(_, x)) => *x,
                            _ => 100.0,
                        },
                        _ => 100.0,
                    };
                    header.set("props", percent_props(&layers, percent, false));
                } else {
                    header.set("props", props_value(&layers));
                }
            } else {
                header.set("props", props_value(&layers));
            }
        }
        e.obj.set("element", V::Obj(header));
    }
    project_rest(scene, input, limits, &oo)
}

/// The projection stages after the element headers.
fn project_rest<'a>(
    mut scene: CanonScene<'a>,
    input: &[u8],
    limits: &DumpLimits,
    oo: &[(usize, usize)],
) -> Result<CanonScene<'a>, String> {
    let links = project_graph(&mut scene, oo);
    project_globals(&mut scene, input, limits)?;
    project_transforms(&mut scene, &links, oo, limits)?;
    if !scene.lean {
        // Take bounds and the animation stack structure only feed the
        // canonical records.
        project_takes(&mut scene, limits)?;
    }
    project_meshes(&mut scene);
    crate::scene_deform::project(&mut scene, limits)?;
    project_materials(&mut scene, limits)?;
    project_attributes(&mut scene, &links, limits)?;
    crate::scene_animation::project(&mut scene, limits)?;
    Ok(scene)
}

/// Lights, cameras and bones (`ufbxi_update_scene` after
/// `ufbxi_postprocess_scene`, so the final metadata units apply; see
/// `scene_attribute`).
fn project_attributes(
    scene: &mut CanonScene<'_>,
    links: &Links,
    limits: &DumpLimits,
) -> Result<(), String> {
    use crate::scene_attribute as sa;
    use crate::scene_node as sn;
    use ElementType::*;
    let f = |name: &str, def: f64| match scene.metadata.get(name) {
        Some(V::F(x)) => *x,
        _ => def,
    };
    let units = sa::SceneUnits {
        geometry_scale: f("geometry_scale", 1.0),
        ortho_size_unit: f("ortho_size_unit", 30.0),
        bone_prop_size_unit: f("bone_prop_size_unit", 1.0),
        bone_prop_limb_length_relative: !matches!(
            scene.metadata.get("bone_prop_limb_length_relative"),
            Some(V::Bool(false))
        ),
    };
    // `ufbxi_update_adjust_transforms`: single-attribute nodes take the
    // target light/camera axes (import profile: both right-handed Y-up).
    const TARGET: [i64; 3] = [0, 2, 4];
    let light_dir = sa::light_direction(TARGET);
    let camera_axes = sn::axis_matrix(TARGET, [4, 2, 1]).map(|_| TARGET);
    let mut direction = vec![sa::DEFAULT_LIGHT_DIRECTION; scene.elements.len()];
    let mut axes = vec![sa::DEFAULT_CAMERA_AXES; scene.elements.len()];
    for attribs in &links.attribs {
        if let [a] = attribs[..] {
            match scene.elements[a].ty {
                Light => direction[a] = light_dir.unwrap_or(direction[a]),
                Camera => axes[a] = camera_axes.unwrap_or(axes[a]),
                _ => {}
            }
        }
    }
    for i in 0..scene.elements.len() {
        let (ty @ (Light | Camera | Bone), Some(node)) =
            (scene.elements[i].ty, scene.elements[i].node)
        else {
            continue;
        };
        let props = scene_property::layers(
            &scene.doc,
            node,
            limits.max_properties,
            limits.max_allocation,
        )
        .map_err(|err| format!("attribute properties: {err:?}"))?;
        let record = match ty {
            Light if scene.lean => {
                scene
                    .typed
                    .lights
                    .push((i, sa::light(&props, direction[i])));
                continue;
            }
            Camera if scene.lean => {
                let camera = sa::camera(&props, &units, axes[i]);
                scene.typed.cameras.push((i, camera));
                continue;
            }
            Light => sa::light(&props, direction[i]).to_obj(),
            Camera => sa::camera(&props, &units, axes[i]).to_obj(),
            _ => {
                let sub = string_of(props_of(&scene.doc, node).get(2));
                sa::bone(&props, &units, sub == b"Root")
            }
        };
        for (name, v) in record.0 {
            scene.elements[i].obj.set(name, v);
        }
    }
    Ok(())
}

/// Destination elements of `e`'s object-object source connections, in ufbx
/// `connections_src` order (`ufbxi_find_src_connections(element, NULL)`).
fn src_object_connections(e: &Element) -> Vec<usize> {
    e.src_conns
        .iter()
        .filter(|(_, prop)| prop.is_empty())
        .map(|&(dst, _)| dst)
        .collect()
}

/// `ufbx_material`, `ufbx_texture`, `ufbx_video` records and
/// `scene.texture_files` (see `scene_material`).
fn project_materials(scene: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    use crate::scene_material::{self as sm, Env, Item};
    use ElementType::*;
    let mut children = vec![Vec::new(); scene.doc.nodes.len()];
    for (i, node) in scene.doc.nodes.iter().enumerate() {
        if let Some(p) = node.parent {
            children[p].push(i);
        }
    }
    let types: Vec<ElementType> = scene.elements.iter().map(|e| e.ty).collect();
    let mut items = Vec::new();
    for (i, e) in scene.elements.iter().enumerate() {
        let (Material | Texture | Video, Some(node)) = (e.ty, e.node) else {
            continue;
        };
        let props = scene_property::layers(
            &scene.doc,
            node,
            limits.max_properties,
            limits.max_allocation,
        )
        .map_err(|err| format!("material properties: {err:?}"))?;
        items.push(Item {
            element: i,
            ty: e.ty,
            node,
            props,
            dst: dst_connections(e),
            src: src_object_connections(e),
        });
    }
    if items.is_empty() {
        return Ok(());
    }
    let env = Env {
        doc: &scene.doc,
        children: &children,
        ascii: scene.ascii,
        original_file_path: scene.original_file_path.clone(),
        types: &types,
        lean: scene.lean,
    };
    let out = sm::project(&env, &items);
    drop(items);
    scene.warnings.extend(out.warnings);
    scene.typed.materials = out.materials;
    scene.typed.textures = out.textures;
    for (i, record) in out.records {
        let obj = &mut scene.elements[i].obj;
        for (name, v) in record.0 {
            obj.set(name, v);
        }
    }
    scene.texture_files = out.texture_files;
    Ok(())
}

/// Node hierarchy and attribute links from object-object connections
/// (ufbx `ufbxi_finalize_scene` node linking; order follows connections).
fn project_graph(scene: &mut CanonScene<'_>, oo: &[(usize, usize)]) -> Links {
    use ElementType::*;
    let n = scene.elements.len();
    let mut parent = vec![None; n];
    let mut children = vec![Vec::new(); n];
    let mut attribs = vec![Vec::new(); n];
    let mut instances = vec![Vec::new(); n];
    let mut materials = vec![Vec::new(); n];
    for &(src, dst) in oo {
        let (st, dt) = (scene.elements[src].ty, scene.elements[dst].ty);
        if dt != Node {
            continue;
        }
        if st == Material {
            materials[dst].push(src);
        } else if st == Node {
            if parent[src].is_none() && src != 0 {
                parent[src] = Some(dst);
                children[dst].push(src);
            }
        } else if (Mesh..=LodGroup).contains(&st) {
            attribs[dst].push(src);
            instances[src].push(dst);
        }
    }
    // ufbxi_cmp_node_less puts scale helpers before ordinary siblings.
    for list in &mut children {
        list.sort_by_key(|i| !scene.scale_helpers.iter().any(|&(_, h)| h == *i));
    }
    let mut depth = vec![0u64; n];
    for i in 0..n {
        let mut d = 0;
        let mut p = parent[i];
        while let Some(q) = p {
            d += 1;
            if d > n as u64 {
                break;
            }
            p = parent[q];
        }
        depth[i] = d;
    }
    let effective_parent: Vec<Option<usize>> = (0..n)
        .map(|i| parent[i].or(if i == 0 { None } else { Some(0) }))
        .collect();
    for (e, inst) in scene.elements.iter_mut().zip(&mut instances) {
        if (Mesh..=LodGroup).contains(&e.ty) {
            e.instances = std::mem::take(inst);
        }
    }
    if scene.lean {
        let t = &mut scene.typed;
        t.parent = effective_parent.clone();
        t.children = children;
        t.node_materials = materials;
        t.attribs = attribs.clone();
        return Links {
            parent: effective_parent,
            depth: (0..n)
                .map(|i| depth[i] + u64::from(i != 0 && parent[i].is_none()))
                .collect(),
            attribs,
        };
    }
    for i in 0..n {
        let ty = scene.elements[i].ty;
        if (Mesh..=LodGroup).contains(&ty) {
            let instances = &scene.elements[i].instances;
            let list = V::List(instances.iter().map(|&k| V::Ref(Some(k))).collect());
            if let Some(V::Obj(h)) = scene.elements[i].obj.get_mut("element") {
                h.set("instances", list);
            }
        }
        if ty != Node {
            continue;
        }
        let first = attribs[i].first().copied();
        let first_ty = first.map(|a| scene.elements[a].ty);
        let obj = &mut scene.elements[i].obj;
        obj.set(
            "parent",
            V::Ref(parent[i].or(if i == 0 { None } else { Some(0) })),
        );
        obj.set(
            "children",
            V::List(children[i].iter().map(|&k| V::Ref(Some(k))).collect()),
        );
        obj.set(
            "node_depth",
            V::U(depth[i] + u64::from(i != 0 && parent[i].is_none())),
        );
        obj.set("is_root", V::Bool(i == 0));
        obj.set(
            "materials",
            V::List(materials[i].iter().map(|&k| V::Ref(Some(k))).collect()),
        );
        if i == 0 {
            // The synthetic root carries identity transforms (ufbx root node).
            const ID: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
            let quat = || V::Vec(vec![0.0, 0.0, 0.0, 1.0]);
            let tr = || {
                V::Obj(
                    Obj::new()
                        .with("rotation", V::Vec(vec![0.0, 0.0, 0.0, 1.0]))
                        .with("scale", V::Vec(vec![1.0; 3])),
                )
            };
            obj.set("local_transform", tr());
            obj.set("geometry_transform", tr());
            for m in [
                "node_to_parent",
                "node_to_world",
                "geometry_to_node",
                "geometry_to_world",
                "unscaled_node_to_world",
            ] {
                obj.set(m, V::Vec(ID.to_vec()));
            }
            obj.set("adjust_pre_rotation", quat());
            obj.set("adjust_post_rotation", quat());
            for f in [
                "adjust_pre_scale",
                "adjust_post_scale",
                "adjust_translation_scale",
            ] {
                obj.set(f, V::F(1.0));
            }
            obj.set("inherit_scale", V::Vec(vec![1.0; 3]));
            obj.set("visible", V::Bool(true));
        }
        obj.set("attrib", V::Ref(first));
        obj.set(
            "all_attribs",
            V::List(attribs[i].iter().map(|&k| V::Ref(Some(k))).collect()),
        );
        if let (Some(a), Some(t)) = (first, first_ty) {
            obj.set("attrib_type", V::I(t as i64));
            let field = match t {
                Mesh => Some("mesh"),
                Light => Some("light"),
                Camera => Some("camera"),
                Bone => Some("bone"),
                _ => None,
            };
            if let Some(f) = field {
                obj.set(f, V::Ref(Some(a)));
            }
        }
    }
    Links {
        parent: (0..n)
            .map(|i| parent[i].or(if i == 0 { None } else { Some(0) }))
            .collect(),
        depth: (0..n)
            .map(|i| depth[i] + u64::from(i != 0 && parent[i].is_none()))
            .collect(),
        attribs,
    }
}

/// Node hierarchy facts shared by the graph and transform projections.
struct Links {
    /// Effective parent (top-level nodes hang from the synthetic root 0).
    parent: Vec<Option<usize>>,
    /// `ufbx_node.node_depth`.
    depth: Vec<u64>,
    /// Attribute elements in connection order.
    attribs: Vec<Vec<usize>>,
}

/// Node transforms under the import profile options (see `scene_node` for the ported
/// arithmetic): `geometry_transform_handling = MODIFY_GEOMETRY`,
/// `inherit_mode_handling = COMPENSATE`, `space_conversion = MODIFY_GEOMETRY`,
/// target axes right-handed Y-up, `target_unit_meters = 1`, mirror axis Z,
/// target camera and light axes right-handed Y-up.
fn project_transforms(
    scene: &mut CanonScene<'_>,
    links: &Links,
    oo: &[(usize, usize)],
    limits: &DumpLimits,
) -> Result<(), String> {
    use crate::scene_node::{self as sn, Adjust, NodeProps};
    use ElementType::*;
    let n = scene.elements.len();
    let is_node: Vec<bool> = scene.elements.iter().map(|e| e.ty == Node).collect();
    // Properties through the template chain (`ufbxi_find_*`).
    let mut layers = Vec::with_capacity(n);
    for e in &scene.elements {
        layers.push(match (e.ty, e.node) {
            (Node, Some(d)) => Some(crate::scene_animation::node_layers(
                &scene.doc,
                d,
                scene.scale_helpers.iter().any(|&(_, h)| h == layers.len()),
                scene.scale_helpers.iter().any(|&(o, _)| o == layers.len()),
                limits,
            )?),
            _ => None,
        });
    }
    let find = |i: usize, name: &[u8]| layers[i].as_ref().and_then(|l| l.find(name));
    let vec3 = |i: usize, name: &[u8], d: [f64; 3]| {
        find(i, name).map_or(d, |p| [p.value_real[0], p.value_real[1], p.value_real[2]])
    };
    let int = |i: usize, name: &[u8], d: i64| find(i, name).map_or(d, |p| p.value_int);

    // Source axes and units (`ufbxi_update_scene_settings`).
    let doc = &scene.doc;
    let mut global = Vec::new();
    if let Some(g) = doc
        .nodes
        .iter()
        .position(|x| x.parent.is_none() && x.name == b"GlobalSettings")
    {
        global = scene_property::authored(doc, g, limits.max_properties, limits.max_allocation)
            .map_err(|e| format!("global properties: {e:?}"))?;
    }
    let gfind = |name: &[u8]| global.iter().find(|p| p.name == name);
    let gint = |name: &[u8], d: i64| gfind(name).map_or(d, |p| p.value_int);
    let axis = |a: &[u8], s: &[u8]| {
        let (a, s) = (gint(a, 3), gint(s, 2));
        match a {
            0..=2 => a * 2 + i64::from(s <= 0),
            _ => 6,
        }
    };
    let src_axes = [
        axis(b"CoordAxis", b"CoordAxisSign"),
        axis(b"UpAxis", b"UpAxisSign"),
        axis(b"FrontAxis", b"FrontAxisSign"),
    ];
    let unit_factor = gfind(b"UnitScaleFactor").map_or(1.0, |p| p.value_real[0]);
    let unit_meters = sn::round_if_near(unit_factor * 0.01);

    // `ufbxi_transform_to_axes`.
    const TARGET: [i64; 3] = [0, 2, 4];
    let mut axis_matrix = [0.0; 12];
    let mut mirror_axis = 0;
    if sn::axes_valid(src_axes) {
        if let Some(m) = sn::axis_matrix(src_axes, TARGET) {
            axis_matrix = m;
            if sn::matrix_determinant(&axis_matrix) < 0.0 {
                mirror_axis = 3;
                sn::mirror_matrix_dst(&mut axis_matrix, mirror_axis);
            }
        }
    }
    // `ufbxi_scale_units`.
    let mut unit_scale = 1.0;
    if unit_meters > 0.0 {
        let ratio = sn::round_if_near(unit_meters / sn::round_if_near(1.0));
        if ratio != 1.0 {
            unit_scale = ratio;
        }
    }

    // Inherit modes (`ufbxi_read_model`, COMPENSATE leaves `inherit_mode` NORMAL).
    let original: Vec<i64> = (0..n)
        .map(|i| match int(i, b"InheritType", -1) {
            0 if is_node[i] => 2,
            2 if is_node[i] => 1,
            _ => 0,
        })
        .collect();
    // `ufbxi_pre_finalize_scene`: scale compensation for unscaled children.
    let mut unscaled_children = vec![false; n];
    for &(src, dst) in oo {
        if is_node[src] && is_node[dst] && dst != 0 && original[src] != 0 {
            unscaled_children[dst] = true;
        }
    }
    let mut compensate_parent = vec![false; n];
    for i in 0..n {
        if !unscaled_children[i] {
            continue;
        }
        let scale = vec3(i, b"Lcl Scaling", [1.0; 3]);
        let r = scale[0];
        let (dx, dy, dz) = (
            (scale[0] - r).abs(),
            (scale[1] - r).abs(),
            (scale[2] - r).abs(),
        );
        if dx + dy + dz >= 0.001_f32 as f64 || scale[0].abs() <= 0.01_f32 as f64 {
            // ufbx inserts a scale helper node here (`ufbxi_setup_scale_helper`).
            continue;
        }
        if (scale[0] - 1.0).abs() >= 0.001_f32 as f64 {
            compensate_parent[i] = true;
        }
    }

    // `ufbxi_update_adjust_transforms`.
    let mut root = sn::IDENTITY_TRANSFORM;
    if axis_matrix.iter().any(|&x| x != 0.0) {
        root = sn::matrix_to_transform(&axis_matrix);
    }
    for s in root.scale.iter_mut() {
        *s *= unit_scale;
    }
    let root_scale = {
        let m = |a: f64, b: f64| if a < b { a } else { b };
        m(m(root.scale[0], root.scale[1]), root.scale[2])
    };
    let post_rotation =
        |src: [i64; 3]| sn::axis_matrix(TARGET, src).map(|m| sn::matrix_to_transform(&m).rotation);
    let light_post = post_rotation([0, 5, 2]);
    let camera_post = post_rotation([4, 2, 1]);
    let order: Vec<usize> = {
        let mut v: Vec<usize> = (0..n).filter(|&i| is_node[i]).collect();
        v.sort_by_key(|&i| {
            (
                links.depth[i],
                !scene.scale_helpers.iter().any(|&(_, h)| h == i),
            )
        });
        v
    };
    let mut adjust = vec![Adjust::default(); n];
    let mut inherit_node: Vec<Option<usize>> = vec![None; n];
    for &i in &order {
        let mut a = Adjust::default();
        if i != 0 {
            a.mirror_axis = mirror_axis;
            if links.depth[i] <= 1 {
                a.pre_rotation = root.rotation;
            }
            a.translation_scale = root_scale;
            a.has_adjust_transform = true;
        }
        if let Some(p) = links.parent[i] {
            // `has_root_adjust_transform` is only set under ADJUST_TRANSFORMS.
            if compensate_parent[p] && original[i] == 1 {
                let scale = vec3(p, b"Lcl Scaling", [1.0; 3]);
                let mut size = scale[0];
                if (scale[1] - 1.0).abs() < (size - 1.0).abs() {
                    size = scale[1];
                }
                if (scale[2] - 1.0).abs() < (size - 1.0).abs() {
                    size = scale[2];
                }
                a.post_scale *= 1.0 / size;
                a.has_adjust_transform = true;
            }
            inherit_node[i] = match original[i] {
                2 => Some(p),
                1 => inherit_node[p],
                _ => None,
            };
        }
        if links.attribs[i].len() == 1 {
            let ty = scene.elements[links.attribs[i][0]].ty;
            if let (Light, Some(q)) | (Camera, Some(q)) =
                (ty, if ty == Light { light_post } else { camera_post })
            {
                a.post_rotation = q;
                a.has_adjust_transform = true;
            }
        }
        adjust[i] = a;
    }

    // `ufbxi_update_node` (geometric properties were reset by
    // `ufbxi_modify_geometry`, so every node uses the identity defaults).
    let mut states: Vec<Option<sn::NodeState>> = vec![None; n];
    let mut eval_nodes = Vec::new();
    let mut eval_slot = vec![None; n];
    for i in 0..n {
        if is_node[i] {
            eval_slot[i] = Some(eval_nodes.len());
            eval_nodes.push(crate::animation::NodeEval {
                element: i,
                parent: None,
                props: NodeProps::default(),
                adjust: adjust[i],
                rotation_order: 0,
                use_rotation_space: false,
                translation_scale: None,
                inherited_scale: None,
            });
        }
    }
    scene.animation.node_order = order.iter().filter_map(|&i| eval_slot[i]).collect();
    if scene.lean {
        scene.typed.visible = vec![true; n];
    }
    for &i in &order {
        let rotation_order = {
            let v = int(i, b"RotationOrder", 0);
            if (0..=6).contains(&v) {
                v
            } else {
                0
            }
        };
        let euler = vec3(i, b"Lcl Rotation", [0.0; 3]);
        let a = &adjust[i];
        let (state, use_rotation_space) = if i == 0 {
            (sn::root_state(), false)
        } else {
            let use_rs =
                int(i, b"RotationActive", 1) != 0 && int(i, b"RotationSpaceForLimitOnly", 0) == 0;
            let p = NodeProps {
                translation: vec3(i, b"Lcl Translation", [0.0; 3]),
                rotation: euler,
                scaling: vec3(i, b"Lcl Scaling", [1.0; 3]),
                pre_rotation: vec3(i, b"PreRotation", [0.0; 3]),
                post_rotation: vec3(i, b"PostRotation", [0.0; 3]),
                rotation_offset: vec3(i, b"RotationOffset", [0.0; 3]),
                rotation_pivot: vec3(i, b"RotationPivot", [0.0; 3]),
                scaling_offset: vec3(i, b"ScalingOffset", [0.0; 3]),
                scaling_pivot: vec3(i, b"ScalingPivot", [0.0; 3]),
                ..NodeProps::default()
            };
            if let Some(slot) = eval_slot[i] {
                eval_nodes[slot] = crate::animation::NodeEval {
                    element: i,
                    parent: links.parent[i].and_then(|p| eval_slot[p]),
                    props: p,
                    adjust: *a,
                    rotation_order,
                    use_rotation_space: use_rs,
                    translation_scale: links.parent[i].and_then(|p| {
                        scene
                            .scale_helpers
                            .iter()
                            .find(|&&(o, h)| o == p && h != i)
                            .and_then(|&(_, h)| eval_slot[h])
                    }),
                    inherited_scale: scene
                        .scale_helpers
                        .iter()
                        .any(|&(_, h)| h == i)
                        .then(|| {
                            links.parent[i].and_then(|p| inherit_node[p]).and_then(|p| {
                                scene
                                    .scale_helpers
                                    .iter()
                                    .find(|&&(o, _)| o == p)
                                    .and_then(|&(_, h)| eval_slot[h])
                            })
                        })
                        .flatten(),
                };
            }
            let config = &eval_nodes[eval_slot[i].unwrap()];
            let translation_scale = config.translation_scale.and_then(|slot| {
                states[eval_nodes[slot].element]
                    .as_ref()
                    .map(|s| s.local.scale)
            });
            let mut local = sn::get_transform(&p, rotation_order, use_rs, a, translation_scale);
            if let Some(slot) = config.inherited_scale {
                if let Some(st) = &states[eval_nodes[slot].element] {
                    for c in 0..3 {
                        local.scale[c] *= st.local.scale[c];
                    }
                }
            }
            let geometry = sn::get_geometry_transform(&p, a);
            let parent = links.parent[i].and_then(|q| states[q].as_ref());
            (sn::update_node(local, geometry, parent), use_rs)
        };
        let visible = int(i, b"Visibility", 1) != 0;
        if scene.lean {
            scene.typed.visible[i] = visible;
            states[i] = Some(state);
            continue;
        }
        let obj = &mut scene.elements[i].obj;
        let tr = |t: &sn::Transform| {
            V::Obj(
                Obj::new()
                    .with("translation", V::Vec(t.translation.to_vec()))
                    .with("rotation", V::Vec(t.rotation.to_vec()))
                    .with("scale", V::Vec(t.scale.to_vec())),
            )
        };
        obj.set("inherit_mode", V::I(0));
        if let Some(&(_, h)) = scene.scale_helpers.iter().find(|&&(o, _)| o == i) {
            obj.set("scale_helper", V::Ref(Some(h)));
        }
        obj.set(
            "is_scale_helper",
            V::Bool(scene.scale_helpers.iter().any(|&(_, h)| h == i)),
        );
        obj.set("original_inherit_mode", V::I(original[i]));
        obj.set("local_transform", tr(&state.local));
        obj.set("geometry_transform", tr(&state.geometry));
        obj.set("inherit_scale", V::Vec(state.inherit_scale.to_vec()));
        obj.set("inherit_scale_node", V::Ref(inherit_node[i]));
        obj.set("rotation_order", V::I(rotation_order));
        obj.set("euler_rotation", V::Vec(euler.to_vec()));
        obj.set("node_to_parent", V::Vec(state.node_to_parent.to_vec()));
        obj.set("node_to_world", V::Vec(state.node_to_world.to_vec()));
        obj.set("geometry_to_node", V::Vec(state.geometry_to_node.to_vec()));
        obj.set(
            "geometry_to_world",
            V::Vec(state.geometry_to_world.to_vec()),
        );
        obj.set(
            "unscaled_node_to_world",
            V::Vec(state.unscaled_node_to_world.to_vec()),
        );
        obj.set("adjust_pre_translation", V::Vec(a.pre_translation.to_vec()));
        obj.set("adjust_pre_rotation", V::Vec(a.pre_rotation.to_vec()));
        obj.set("adjust_pre_scale", V::F(a.pre_scale));
        obj.set("adjust_post_rotation", V::Vec(a.post_rotation.to_vec()));
        obj.set("adjust_post_scale", V::F(a.post_scale));
        obj.set("adjust_translation_scale", V::F(a.translation_scale));
        obj.set("adjust_mirror_axis", V::I(a.mirror_axis));
        obj.set("visible", V::Bool(visible));
        obj.set(
            "has_geometry_transform",
            V::Bool(state.has_geometry_transform),
        );
        obj.set("use_rotation_space", V::Bool(use_rotation_space));
        obj.set("has_adjust_transform", V::Bool(a.has_adjust_transform));
        obj.set(
            "has_root_adjust_transform",
            V::Bool(a.has_root_adjust_transform),
        );
        obj.set("is_scale_compensate_parent", V::Bool(compensate_parent[i]));
        states[i] = Some(state);
    }
    scene
        .metadata
        .set("root_rotation", V::Vec(root.rotation.to_vec()));
    scene.metadata.set("root_scale", V::F(1.0));
    scene.metadata.set("geometry_scale", V::F(root_scale));
    scene.metadata.set("mirror_axis", V::I(mirror_axis));
    // End of `ufbxi_modify_geometry`: the Blender binary exporter's
    // orthographic size follows the geometry scale.
    let ortho_size_unit = if scene.exporter == EXPORTER_BLENDER_BINARY {
        1.0 / root_scale
    } else {
        30.0
    };
    scene.metadata.set("ortho_size_unit", V::F(ortho_size_unit));
    scene.node_states = states;
    scene.animation.nodes = eval_nodes;
    // `ufbxi_modify_geometry` prefetch: geometric transforms of every node.
    let mut geometry_modify = vec![None; n];
    for &i in &order {
        if i == 0 {
            continue;
        }
        let p = NodeProps {
            geometric_translation: vec3(i, b"GeometricTranslation", [0.0; 3]),
            geometric_rotation: vec3(i, b"GeometricRotation", [0.0; 3]),
            geometric_scaling: vec3(i, b"GeometricScaling", [1.0; 3]),
            ..NodeProps::default()
        };
        let t = sn::get_geometry_transform(&p, &adjust[i]);
        if !sn::is_transform_identity(&t) {
            geometry_modify[i] = Some(sn::transform_to_matrix(&t));
        }
    }
    scene.geometry_modify = geometry_modify;
    Ok(())
}

/// `connections_dst` of an element in ufbx order (stable by `dst_prop`).
fn dst_connections(e: &Element) -> Vec<(usize, Vec<u8>)> {
    let mut out = e.dst_conns.clone();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// `ufbx_mesh` records (see `scene_geometry`) and the scene's
/// `metadata.max_face_triangles`.
fn project_meshes(scene: &mut CanonScene<'_>) {
    use crate::scene_geometry::{convert_mesh, project_mesh, MeshEnv, MeshLinks};
    use ElementType::*;
    let mut children = vec![Vec::new(); scene.doc.nodes.len()];
    for (i, node) in scene.doc.nodes.iter().enumerate() {
        if let Some(p) = node.parent {
            children[p].push(i);
        }
    }
    let geometry_scale = match scene.metadata.get("geometry_scale") {
        Some(V::F(x)) => *x,
        _ => 1.0,
    };
    let mirror_axis = match scene.metadata.get("mirror_axis") {
        Some(V::I(x)) => *x,
        _ => 0,
    };
    let mut jobs = Vec::new();
    for (i, e) in scene.elements.iter().enumerate() {
        let (Mesh, Some(node)) = (e.ty, e.node) else {
            continue;
        };
        let instances = &e.instances;
        let ty = |k: usize| scene.elements[k].ty;
        let mut searched = vec![dst_connections(e)];
        if let Some(&f) = instances.first() {
            searched.push(dst_connections(&scene.elements[f]));
        }
        let mut links = MeshLinks::default();
        for conns in &searched {
            links.materials = conns
                .iter()
                .filter(|(s, p)| p.is_empty() && ty(*s) == Material)
                .map(|(s, _)| *s)
                .collect();
            if !links.materials.is_empty() {
                break;
            }
        }
        let fetch = |want: ElementType| {
            let mut out: Vec<usize> = Vec::new();
            for conns in &searched {
                for (s, p) in conns {
                    if p.is_empty() && ty(*s) == want && !out.contains(s) {
                        out.push(*s);
                    }
                }
            }
            out
        };
        links.skin_deformers = fetch(SkinDeformer);
        links.blend_deformers = fetch(BlendDeformer);
        links.cache_deformers = fetch(CacheDeformer);
        for conns in &searched {
            for (s, _) in conns {
                if matches!(ty(*s), SkinDeformer | BlendDeformer | CacheDeformer) {
                    links.all_deformers.push(*s);
                }
            }
        }
        if instances.len() == 1 {
            links.geometry_to_node = scene.geometry_modify.get(instances[0]).copied().flatten();
        }
        jobs.push((i, node, links));
    }
    let env = MeshEnv {
        doc: &scene.doc,
        children: &children,
        ascii: scene.ascii,
        geometry_scale,
        mirror_axis,
    };
    let mut max_face_triangles = 0;
    for (i, node, links) in jobs {
        if scene.lean {
            match convert_mesh(&env, node, &links) {
                Ok(m) => scene.typed.meshes.push((i, m, links.materials)),
                Err(e) => scene
                    .warnings
                    .push(format!("{}: {e}", scene.elements[i].key)),
            }
            continue;
        }
        let obj = &mut scene.elements[i].obj;
        match project_mesh(&env, node, &links, obj) {
            Ok(t) => max_face_triangles = max_face_triangles.max(t),
            Err(e) => scene
                .warnings
                .push(format!("{}: {e}", scene.elements[i].key)),
        }
    }
    if scene.lean {
        return;
    }
    scene
        .metadata
        .set("max_face_triangles", V::U(max_face_triangles));
}

/// `ufbx_exporter` values (ufbx.h 0.23.1).
const EXPORTER_UNKNOWN: i64 = 0;
const EXPORTER_FBX_SDK: i64 = 1;
const EXPORTER_BLENDER_BINARY: i64 = 2;
const EXPORTER_BLENDER_ASCII: i64 = 3;
const EXPORTER_MOTION_BUILDER: i64 = 4;
const EXPORTER_UFBX_WRITE: i64 = 5;

/// `ufbxi_match_version_string`: lowercase letters match either case, `' '`
/// skips blanks, `'-'` skips past the next `-`, punctuation matches itself and
/// `'?'` reads a decimal number (wrapping `uint32_t` arithmetic). Prefix match.
fn match_version_string(fmt: &[u8], s: &[u8], version: &mut [u32; 3]) -> bool {
    let (mut pos, mut ix) = (0usize, 0usize);
    for &c in fmt {
        match c {
            b'a'..=b'z' => {
                match s.get(pos) {
                    Some(&b) if b == c || i32::from(b as i8) + 32 == i32::from(c) => {}
                    _ => return false,
                }
                pos += 1;
            }
            b' ' => {
                while matches!(s.get(pos), Some(b' ' | b'\t')) {
                    pos += 1;
                }
            }
            b'-' => {
                while s.get(pos).is_some_and(|&b| b != b'-') {
                    pos += 1;
                }
                if pos >= s.len() {
                    return false;
                }
                pos += 1;
            }
            b'?' => {
                let start = pos;
                let mut num = 0u32;
                while let Some(&b) = s.get(pos).filter(|b| b.is_ascii_digit()) {
                    num = num.wrapping_mul(10).wrapping_add(u32::from(b - b'0'));
                    pos += 1;
                }
                if pos == start {
                    return false;
                }
                version[ix] = num;
                ix += 1;
            }
            _ => {
                if s.get(pos) != Some(&c) {
                    return false;
                }
                pos += 1;
            }
        }
    }
    true
}

/// `ufbx_pack_version` (unsigned 32-bit arithmetic).
fn pack_version(major: u32, minor: u32, patch: u32) -> u32 {
    major
        .wrapping_mul(1_000_000)
        .wrapping_add(minor.wrapping_mul(1000))
        .wrapping_add(patch)
}

/// `ufbxi_match_exporter`: (exporter, exporter_version) from the creator
/// string, keeping `detected` (the ASCII magic-comment Blender detection)
/// when no pattern matches.
fn match_exporter(creator: &[u8], detected: i64) -> (i64, u32) {
    let mut v = [0u32; 3];
    if match_version_string(b"blender-- ?.?.?", creator, &mut v) {
        (EXPORTER_BLENDER_BINARY, pack_version(v[0], v[1], v[2]))
    } else if match_version_string(b"blender- ?.?", creator, &mut v) {
        (EXPORTER_BLENDER_BINARY, pack_version(v[0], v[1], 0))
    } else if match_version_string(b"blender version ?.?", creator, &mut v) {
        (EXPORTER_BLENDER_ASCII, pack_version(v[0], v[1], 0))
    } else if match_version_string(b"fbx sdk/fbx plugins version ?.?", creator, &mut v) {
        (EXPORTER_FBX_SDK, pack_version(v[0], v[1], 0))
    } else if match_version_string(b"fbx sdk/fbx plugins build ?", creator, &mut v) {
        (
            EXPORTER_FBX_SDK,
            pack_version(v[0] / 10000, v[0] / 100 % 100, v[0] % 100),
        )
    } else if match_version_string(b"motionbuilder version ?.?", creator, &mut v)
        || match_version_string(b"motionbuilder/mocap/online version ?.?", creator, &mut v)
    {
        (EXPORTER_MOTION_BUILDER, pack_version(v[0], v[1], 0))
    } else if match_version_string(b"ufbx_write", creator, &mut v) {
        (EXPORTER_UFBX_WRITE, pack_version(0, 0, 1))
    } else {
        (detected, 0)
    }
}

/// `ufbxi_ascii_skip_whitespace` first-comment handling: when the leading
/// comment is the `; FBX d.d.d` magic, a second line starting
/// `; Created by Blender` marks the Blender 6100 ASCII exporter.
fn ascii_blender_comment(input: &[u8]) -> bool {
    let mut pos = usize::from(input.starts_with(b"\xef\xbb\xbf")) * 3;
    let byte = |i: usize| input.get(i).copied().unwrap_or(0);
    while matches!(byte(pos), b' ' | b'\t' | b'\r' | b'\n') {
        pos += 1;
    }
    if byte(pos) != b';' {
        return false;
    }
    // `ufbxi_ascii_parse_version`: " FBX ?.?.?" after the `;`.
    pos += 1;
    while matches!(byte(pos), b' ' | b'\t') {
        pos += 1;
    }
    if !input.get(pos..).is_some_and(|r| r.starts_with(b"FBX")) {
        return false;
    }
    pos += 3;
    while matches!(byte(pos), b' ' | b'\t') {
        pos += 1;
    }
    for (i, want_digit) in [true, false, true, false, true].into_iter().enumerate() {
        let b = byte(pos + i);
        if want_digit != b.is_ascii_digit() || (!want_digit && b != b'.') {
            return false;
        }
    }
    // The parser stops on the character after the last digit; the comment
    // skip then advances once before scanning for the end of the line.
    pos += 6;
    while pos < input.len() && byte(pos) != b'\n' && byte(pos) != 0 {
        pos += 1;
    }
    pos += 1;
    if pos >= input.len() || byte(pos) != b';' {
        return false;
    }
    let line = &input[pos + 1..];
    let end = line
        .iter()
        .position(|&b| b == b'\n' || b == 0)
        .unwrap_or(line.len());
    line[..end.min(32)].starts_with(b" Created by Blender")
}

fn project_globals(
    scene: &mut CanonScene<'_>,
    input: &[u8],
    limits: &DumpLimits,
) -> Result<(), String> {
    let doc = &scene.doc;
    let find_top = |name: &[u8]| {
        doc.nodes
            .iter()
            .position(|n| n.parent.is_none() && n.name == name)
    };
    let mut global_props = Vec::new();
    if let Some(g) = find_top(b"GlobalSettings") {
        global_props =
            scene_property::authored(doc, g, limits.max_properties, limits.max_allocation)
                .map_err(|e| format!("global properties: {e:?}"))?;
        scene.settings.set(
            "props",
            V::Obj(Obj::new().with(
                "props",
                V::Obj(
                    Obj::new().with("data", V::List(global_props.iter().map(prop_obj).collect())),
                ),
            )),
        );
    }
    // ufbxi_update_scene_settings (ufbx.c 0.23.1).
    let find = |name: &[u8]| global_props.iter().find(|p| p.name == name);
    let find_int = |name: &[u8], d: i64| find(name).map_or(d, |p| p.value_int);
    let find_real = |name: &[u8], d: f64| find(name).map_or(d, |p| p.value_real[0]);
    let find_axis = |axis: &[u8], sign: &[u8]| {
        let (a, s) = (find_int(axis, 3), find_int(sign, 2));
        match a {
            0..=2 => a * 2 + i64::from(s <= 0),
            _ => 6,
        }
    };
    let find_enum = |name: &[u8], d: i64, max: i64| {
        let v = find_int(name, d);
        if (0..=max).contains(&v) {
            v
        } else {
            d
        }
    };
    let round_if_near = |value: f64| {
        let targets = [
            0.0, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6,
            1e7, 1e8, 1e9,
        ];
        for target in targets {
            let error = (target * 9.536_743_164_062_5e-7f64)
                .abs()
                .max(7.523_163_845_262_64e-37);
            if value >= target - error && value <= target + error {
                return target;
            }
        }
        value
    };
    const FPS: [f32; 18] = [
        30.0, 120.0, 100.0, 60.0, 50.0, 48.0, 30.0, 30.0, 29.97, 29.97, 25.0, 24.0, 1000.0, 23.976,
        24.0, 96.0, 72.0, 59.94,
    ];
    let unit = find_real(b"UnitScaleFactor", 1.0);
    let original_unit = find_real(b"OriginalUnitScaleFactor", unit);
    let time_mode = find_enum(b"TimeMode", 11, 17);
    let fps = if time_mode != 14 {
        f64::from(FPS[time_mode as usize])
    } else {
        find_real(b"CustomFrameRate", 24.0)
    };
    scene.settings.set("frames_per_second", V::F(fps));
    scene.settings.set("time_mode", V::I(time_mode));
    scene
        .settings
        .set("time_protocol", V::I(find_enum(b"TimeProtocol", 2, 2)));
    scene
        .settings
        .set("snap_mode", V::I(find_enum(b"SnapOnFrameMode", 0, 3)));
    scene.settings.set(
        "ambient_color",
        V::Vec(find(b"AmbientColor").map_or(vec![0.0; 3], |p| p.value_real[..3].to_vec())),
    );
    scene.settings.set(
        "default_camera",
        s(find(b"DefaultCamera").map_or(&b""[..], |p| p.value_str)),
    );
    scene.settings.set(
        "original_axis_up",
        V::I(find_axis(b"OriginalUpAxis", b"OriginalUpAxisSign")),
    );
    scene.settings.set(
        "original_unit_meters",
        V::F(round_if_near(original_unit * 0.01)),
    );
    // ufbxi_transform_to_axes leaves unknown source axes unchanged.
    let source_axes = [
        find_axis(b"CoordAxis", b"CoordAxisSign"),
        find_axis(b"UpAxis", b"UpAxisSign"),
        find_axis(b"FrontAxis", b"FrontAxisSign"),
    ];
    let axes = if crate::scene_node::axes_valid(source_axes) {
        [0, 2, 4]
    } else {
        source_axes
    };
    // Target axes/units after import profile conversion (right-handed Y-up,
    // metres) when the source coordinate system is known.
    scene.settings.set(
        "axes",
        V::Obj(
            Obj::new()
                .with("right", V::I(axes[0]))
                .with("up", V::I(axes[1]))
                .with("front", V::I(axes[2])),
        ),
    );
    // ufbx keeps the source unit in `settings.unit_meters`; the profile's target
    // unit is applied to the geometry (`ufbxi_scale_units`), not recorded here.
    scene
        .settings
        .set("unit_meters", V::F(round_if_near(unit * 0.01)));
    let mut creator: &[u8] = b"";
    if let Some(h) = find_top(b"FBXHeaderExtension") {
        if let Some(c) = (h + 1..doc.nodes.len())
            .take_while(|&i| doc.nodes[i].parent.is_some())
            .find(|&i| doc.nodes[i].parent == Some(h) && doc.nodes[i].name == b"Creator")
        {
            creator = string_of(props_of(doc, c).first());
        }
    }
    // `ufbxi_read_root`: the Blender ASCII exporter stores its version in a
    // top-level `Creator`; then `ufbxi_match_exporter`. Legacy (< 6000) roots
    // never match, so their metadata exporter stays unknown.
    let detected = if scene.ascii && ascii_blender_comment(input) {
        EXPORTER_BLENDER_ASCII
    } else {
        EXPORTER_UNKNOWN
    };
    let (exporter, exporter_version) = if doc.version >= 6000 {
        if detected == EXPORTER_BLENDER_ASCII {
            if let Some(c) = find_top(b"Creator") {
                if let Some(Property::String(_) | Property::AsciiBare(_)) = props_of(doc, c).first()
                {
                    creator = string_of(props_of(doc, c).first());
                }
            }
        }
        match_exporter(creator, detected)
    } else {
        (EXPORTER_UNKNOWN, 0)
    };
    scene.exporter = if doc.version >= 6000 {
        exporter
    } else {
        detected
    };
    // Maya uses 100/3, Blender binary exactly 33, ASCII (and < 6000) 1.0.
    let bone_prop_size_unit = if doc.version < 6000 {
        1.0
    } else {
        match scene.exporter {
            EXPORTER_BLENDER_BINARY => 33.0,
            EXPORTER_BLENDER_ASCII => 1.0,
            _ => 100.0 / 3.0,
        }
    };
    let mut scene_props = Vec::new();
    if let Some(h) = find_top(b"FBXHeaderExtension") {
        if let Some(si) = (h + 1..doc.nodes.len())
            .find(|&i| doc.nodes[i].parent == Some(h) && doc.nodes[i].name == b"SceneInfo")
        {
            scene_props =
                scene_property::authored(doc, si, limits.max_properties, limits.max_allocation)
                    .map_err(|e| format!("scene info properties: {e:?}"))?;
        }
    }
    // `ufbxi_finalize_scene`: `ufbx_find_string`/`ufbx_find_blob` on
    // `DocumentUrl` (used to derive relative texture and video paths).
    if let Some(p) = scene_props.iter().find(|p| p.name == b"DocumentUrl") {
        scene.original_file_path = (p.value_str.to_vec(), p.value_blob.to_vec());
    }
    // `ufbxi_update_scene_metadata`.
    let app = |prefix: &str| {
        let get = |field: &str| {
            let name = format!("{prefix}|Application{field}");
            let p = scene_props.iter().find(|p| p.name == name.as_bytes());
            s(p.map_or(&b""[..], |p| p.value_str))
        };
        V::Obj(
            Obj::new()
                .with("vendor", get("Vendor"))
                .with("name", get("Name"))
                .with("version", get("Version")),
        )
    };
    scene.metadata = Obj::new()
        .with(
            "scene_props",
            V::Obj(Obj::new().with(
                "props",
                V::Obj(
                    Obj::new().with("data", V::List(scene_props.iter().map(prop_obj).collect())),
                ),
            )),
        )
        .with("ascii", V::Bool(scene.ascii))
        .with("version", V::U(u64::from(doc.version)))
        .with("file_format", V::I(1))
        .with("creator", s(creator))
        .with(
            "big_endian",
            V::Bool(!scene.ascii && input.get(22) == Some(&1)),
        )
        .with("exporter", V::I(exporter))
        .with("exporter_version", V::U(u64::from(exporter_version)))
        .with("original_application", app("Original"))
        .with("latest_application", app("LastSaved"))
        .with("ktime_second", V::I(46_186_158_000))
        .with("space_conversion", V::I(2))
        .with("geometry_transform_handling", V::I(2))
        .with("inherit_mode_handling", V::I(2))
        .with("handedness_conversion_axis", V::I(3))
        .with("has_warning", V::Bools(vec![false; 15]))
        .with("bone_prop_size_unit", V::F(bone_prop_size_unit))
        .with(
            "bone_prop_limb_length_relative",
            V::Bool(scene.exporter != EXPORTER_BLENDER_ASCII),
        )
        // Set from the geometry scale in `project_transforms`.
        .with("ortho_size_unit", V::F(30.0))
        .with("root_rotation", V::Vec(vec![0.0, 0.0, 0.0, 1.0]))
        .with("root_scale", V::F(1.0))
        .with("geometry_scale", V::F(1.0));
    Ok(())
}

#[cfg(feature = "parity")]
const DOM_POLICY: &[u8] = br#"{"scalar_width":"NUMBER folds original scalar width","sanitization":"exposed oracle bits exact; discarded source bits bound by input SHA-256","string_arrays":"pointer-backed entries decoded as byte strings"}"#;

/// Write the complete canonical stream for `input` to `out`. `dumper_sha256`
/// is the producer source binding recorded in the receipt (computed by the
/// parity crate, which owns the source list).
#[cfg(feature = "parity")]
pub fn dump<W: Write>(
    input: &[u8],
    out: W,
    limits: &DumpLimits,
    dumper_sha256: &str,
) -> Result<DumpSummary, String> {
    let scene = project(input, limits)?;
    let pins: serde_json::Value =
        serde_json::from_str(include_str!("../reference/pins.json")).map_err(|e| e.to_string())?;
    let pin = &pins["engine-converted"];
    let receipt = json!({
        "kind": "receipt",
        "schema": "fbx-scene-canon/1",
        "profile": "engine-converted",
        "producer": "rust-scene-v1",
        "source_sha256": pin["ufbx.c"],
        "header_sha256": pin["ufbx.h"],
        "coverage_sha256": format!("{:x}", Sha256::digest(include_bytes!("../reference/coverage-engine-converted.json"))),
        "dumper_sha256": dumper_sha256,
        "input_sha256": format!("{:x}", Sha256::digest(input)),
        "input_length": input.len().to_string(),
        "exit_status": "0",
        "real": "f64",
        "external_open": "denied",
    });
    let w = CanonicalWriter::new(out, &receipt, limits.max_line, limits.max_output)
        .map_err(|e| e.to_string())?;
    let keys: Vec<String> = scene.elements.iter().map(|e| e.key.clone()).collect();
    let mut em = Emitter::new(w, &keys);
    write_all(&mut em, &scene, limits).map_err(|e| e.to_string())?;
    em.w.finish().map_err(|e| e.to_string())?;
    let mut summary = DumpSummary {
        elements: scene.elements.len(),
        unsupported: scene.unsupported.clone(),
        warnings: scene.warnings.clone(),
        ..Default::default()
    };
    for e in &scene.elements {
        if e.ty.needs_later_phase() {
            match summary.later_phase.iter_mut().find(|(t, _)| *t == e.ty) {
                Some(slot) => slot.1 += 1,
                None => summary.later_phase.push((e.ty, 1)),
            }
        }
    }
    Ok(summary)
}

#[cfg(feature = "parity")]
fn write_all<W: Write>(
    em: &mut Emitter<'_, W>,
    scene: &CanonScene<'_>,
    limits: &DumpLimits,
) -> io::Result<()> {
    em.w.import_profile_options()?;
    em.begin("globals")?;
    em.leaf_raw("oracle_dom_policy", |b| b.extend_from_slice(DOM_POLICY))?;
    let m = em.push("settings");
    em.emit_struct("ufbx_scene_settings", Some(&scene.settings))?;
    em.restore(m);
    let m = em.push("metadata");
    em.emit_struct("ufbx_metadata", Some(&scene.metadata))?;
    em.restore(m);
    let m = em.push("animation");
    em.emit_struct("ufbx_anim", Some(&scene.anim))?;
    em.restore(m);
    em.end()?;
    crate::canon_dom::write_dom(em, &scene.doc, scene.ascii)?;
    em.begin("elements")?;
    em.count("length", scene.elements.len())?;
    for (i, e) in scene.elements.iter().enumerate() {
        let m = em.push_index(i);
        em.leaf_here(crate::canon::K::Ref, Some(&V::Ref(Some(i))))?;
        // The root has no source: canon.c writes SIZE_MAX of the 64-bit oracle,
        // independent of this target's pointer width.
        let ordinal = if e.ordinal == usize::MAX {
            u64::MAX
        } else {
            e.ordinal as u64
        };
        em.leaf_raw("source_ordinal", |b| {
            b.extend_from_slice(format!("{{\"u64\":\"{ordinal}\"}}").as_bytes())
        })?;
        if let Some(id) = e.source_id {
            em.leaf_raw("source_id", |b| {
                b.extend_from_slice(format!("{{\"i64\":\"{id}\"}}").as_bytes())
            })?;
        }
        em.restore(m);
    }
    em.count("duplicate_source_ids", duplicate_source_ids(scene))?;
    em.end()?;
    em.begin("connections")?;
    em.emit_struct(
        "ufbx_connection_list",
        Some(&Obj::new().with("data", V::List(scene.connections.clone()))),
    )?;
    em.end()?;
    for domain in [
        "graph",
        "meshes",
        "skins",
        "blend_shapes",
        "materials",
        "cameras_lights",
        "curves",
        "advanced",
    ] {
        em.begin(domain)?;
        let n = scene
            .elements
            .iter()
            .filter(|e| e.ty.domain() == domain)
            .count();
        em.count("length", n)?;
        for e in scene.elements.iter().filter(|e| e.ty.domain() == domain) {
            let m = em.push(&e.key);
            em.emit_struct(e.ty.schema(), Some(&e.obj))?;
            em.restore(m);
        }
        if domain == "materials" {
            let m = em.push("texture_files");
            em.emit_struct(
                "ufbx_texture_file_list",
                Some(&Obj::new().with("data", V::List(scene.texture_files.clone()))),
            )?;
            em.restore(m);
        }
        em.end()?;
    }
    let dl = limits.draft.clone();
    let draft = crate::scene_draft::build(&scene.doc, &dl)
        .map_err(|e| io::Error::other(format!("draft: {e:?}")))?;
    let tl = crate::canonical_writer::TriangleDumpLimits {
        max_allocation: dl.max_allocation.saturating_sub(draft.retained_bytes),
        ..Default::default()
    };
    crate::canonical_writer::write_source_triangles(&mut em.w, &draft.index, &draft.meshes, &tl)?;
    crate::animation_canon::write(em, scene)
}

/// canon.c `duplicate_source_ids`: duplicated signed IDs under Objects.
#[cfg(feature = "parity")]
fn duplicate_source_ids(scene: &CanonScene<'_>) -> usize {
    let doc = &scene.doc;
    let Some(objects) = doc
        .nodes
        .iter()
        .position(|n| n.parent.is_none() && n.name == b"Objects")
    else {
        return 0;
    };
    let mut ids: Vec<i64> = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent == Some(objects))
        .filter_map(|(i, _)| match props_of(doc, i).first() {
            Some(Property::I64(v)) => Some(*v),
            Some(Property::I32(v)) => Some(i64::from(*v)),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids.windows(2).filter(|w| w[0] == w[1]).count()
}

/// Animation stacks used by the canonical evaluator (unstable parity bridge).
#[cfg(feature = "parity")]
pub fn animation_stacks<'a>(scene: &'a CanonScene<'_>) -> &'a [crate::AnimationStack] {
    &scene.animation.stacks
}
/// Animation layers used by the canonical evaluator (unstable parity bridge).
#[cfg(feature = "parity")]
pub fn animation_layers<'a>(scene: &'a CanonScene<'_>) -> &'a [crate::AnimationLayer] {
    &scene.animation.layers
}
/// Animation values used by the canonical evaluator (unstable parity bridge).
#[cfg(feature = "parity")]
pub fn animation_values<'a>(scene: &'a CanonScene<'_>) -> &'a [crate::AnimationValue] {
    &scene.animation.values
}
/// Animation curves used by the canonical evaluator (unstable parity bridge).
#[cfg(feature = "parity")]
pub fn animation_curves<'a>(scene: &'a CanonScene<'_>) -> &'a [crate::AnimationCurve] {
    &scene.animation.curves
}
/// Evaluate the full projection for comparison with the lean public API.
#[cfg(feature = "parity")]
pub fn evaluate_pose(
    scene: &CanonScene<'_>,
    stack: Option<usize>,
    time: f64,
) -> Result<crate::EvaluatedScene, crate::Error> {
    scene.animation.evaluate(stack, time)
}
/// Evaluate one canonical binding for comparison with the public API.
#[cfg(feature = "parity")]
pub fn evaluate_binding(
    scene: &CanonScene<'_>,
    stack: Option<usize>,
    binding: &crate::AnimationBinding,
    time: f64,
) -> Result<crate::EvaluatedProperty, crate::Error> {
    Ok(scene.animation.property(
        scene.animation.stack_layers(stack)?,
        binding.element,
        &binding.property,
        time,
    ))
}
