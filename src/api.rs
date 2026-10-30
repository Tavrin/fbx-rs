//! The public loading API: [`load`] converts an FBX file into an owned
//! [`Scene`] under one import profile ([`LoadOptions`]).
//!
//! Every value is the one ufbx 0.23.1 produces for the same file and options
//! (proved field by field on the static corpus, see `docs/VERIFICATION.md`).

use crate::canon::V;
use crate::canon_scene::{self, CanonScene, DumpLimits, ElementType};
use crate::scene;
use crate::scene_mesh::{self, TriangulationLimits};

pub use crate::scene::UnsupportedKind;

/// Errors returned by [`load`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A structural or evaluation error with a static diagnostic.
    InvalidData(&'static str),
    /// An allocator refused a fallible reservation.
    Allocation,
    /// A checked aggregate resource budget was exhausted.
    ResourceLimit(&'static str),
    /// The input is larger than [`Limits::max_input_bytes`].
    InputTooLarge {
        /// Input size in bytes.
        size: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The requested [`LoadOptions`] are not implemented yet (only the
    /// default profile is); names the first unsupported option.
    UnsupportedOptions(&'static str),
    /// The file is not a supported FBX file (bad syntax, unsupported version
    /// such as 6.1 and older, or a limit exceeded while parsing).
    Parse(String),
    /// The file parsed but its object graph or geometry is invalid, or a
    /// conversion limit was exceeded.
    Convert(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidData(message) => f.write_str(message),
            Error::Allocation => f.write_str("allocation refused"),
            Error::ResourceLimit(stage) => write!(f, "resource limit: {stage}"),
            Error::InputTooLarge { size, limit } => {
                write!(
                    f,
                    "input of {size} bytes exceeds the limit of {limit} bytes"
                )
            }
            Error::UnsupportedOptions(name) => write!(f, "unsupported load option: {name}"),
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::Convert(m) => write!(f, "conversion error: {m}"),
        }
    }
}

impl std::error::Error for Error {}
impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Convert(message)
    }
}
impl From<&'static str> for Error {
    fn from(message: &'static str) -> Self {
        Self::InvalidData(message)
    }
}

/// Resource limits for one [`load_with_limits`] call. Every count is checked
/// before the corresponding allocation; the defaults suit files up to a few
/// hundred MiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximum input size in bytes.
    pub max_input_bytes: usize,
    /// Maximum DOM nesting depth.
    pub max_depth: usize,
    /// Maximum DOM nodes.
    pub max_nodes: usize,
    /// Maximum DOM property values.
    pub max_properties: usize,
    /// Maximum elements of one array property.
    pub max_array_elements: usize,
    /// Maximum bytes produced by decompressing arrays, in total.
    pub max_decompressed_bytes: usize,
    /// Maximum length of one string property.
    pub max_string_length: usize,
    /// Maximum FBX objects.
    pub max_objects: usize,
    /// Maximum keys in one animation curve.
    pub max_animation_keys: usize,
    /// Maximum FBX connections.
    pub max_connections: usize,
    /// Maximum bytes requested by one conversion stage.
    pub max_allocation: usize,
    /// Maximum deformer traversal, lookup and sorting work units.
    pub max_deformer_work: usize,
    /// Maximum animation traversal, lookup and sorting work per stage.
    pub max_animation_work: usize,
}

impl Default for Limits {
    fn default() -> Self {
        let d = DumpLimits::default();
        Self {
            max_input_bytes: 256 << 20,
            max_depth: d.syntax.max_depth,
            max_nodes: d.syntax.max_nodes,
            max_properties: d.syntax.max_properties,
            max_array_elements: d.syntax.max_array_elements,
            max_decompressed_bytes: d.syntax.max_decompressed_bytes,
            max_string_length: d.syntax.max_string_length,
            max_objects: d.index.max_objects,
            max_animation_keys: d.max_keys,
            max_connections: d.index.max_connections,
            max_allocation: d.max_allocation,
            max_deformer_work: d.max_deformer_work,
            max_animation_work: d.max_animation_work,
        }
    }
}

impl Limits {
    fn dump_limits(&self) -> DumpLimits {
        let mut d = DumpLimits {
            max_keys: self.max_animation_keys,
            ..DumpLimits::default()
        };
        d.syntax.max_depth = self.max_depth;
        d.syntax.max_nodes = self.max_nodes;
        d.syntax.max_properties = self.max_properties;
        d.syntax.max_array_elements = self.max_array_elements;
        d.syntax.max_decompressed_bytes = self.max_decompressed_bytes;
        d.syntax.max_string_length = self.max_string_length;
        d.syntax.max_allocation = self.max_allocation;
        d.index.max_objects = self.max_objects;
        d.index.max_connections = self.max_connections;
        d.index.max_allocation = self.max_allocation;
        d.max_properties = self.max_properties;
        d.max_allocation = self.max_allocation;
        d.max_deformer_work = self.max_deformer_work;
        d.max_animation_work = self.max_animation_work;
        d
    }
}

/// A signed coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// +X
    PositiveX,
    /// -X
    NegativeX,
    /// +Y
    PositiveY,
    /// -Y
    NegativeY,
    /// +Z
    PositiveZ,
    /// -Z
    NegativeZ,
}

impl Axis {
    fn from_ufbx(v: i64) -> Option<Self> {
        Some(match v {
            0 => Axis::PositiveX,
            1 => Axis::NegativeX,
            2 => Axis::PositiveY,
            3 => Axis::NegativeY,
            4 => Axis::PositiveZ,
            5 => Axis::NegativeZ,
            _ => return None,
        })
    }
}

/// The right, up and front directions of a coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoordinateAxes {
    /// Direction of "right".
    pub right: Axis,
    /// Direction of "up".
    pub up: Axis,
    /// Direction of "front" (towards the viewer).
    pub front: Axis,
}

impl CoordinateAxes {
    /// Right-handed, Y up: right +X, up +Y, front +Z.
    pub const RIGHT_HANDED_Y_UP: Self = Self {
        right: Axis::PositiveX,
        up: Axis::PositiveY,
        front: Axis::PositiveZ,
    };
}

/// How the scene is converted to the target axes and unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpaceConversion {
    /// Bake the conversion into vertex data and node transforms, leaving no
    /// conversion transform on the root.
    ModifyGeometry,
}

/// How geometric (pivot-only, non-inherited) node transforms are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GeometryTransformHandling {
    /// Bake geometric transforms into the vertex data of single-instance
    /// meshes and into light/camera orientation.
    ModifyGeometry,
}

/// How non-standard scale inheritance (Maya segment scale compensation) is
/// handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InheritModeHandling {
    /// Compensate the parent scale on the child, so every node inherits
    /// normally and world matrices stay exact.
    Compensate,
}

/// The import profile. Only the default profile is implemented today; other
/// values are rejected with [`Error::UnsupportedOptions`] rather than
/// silently ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadOptions {
    /// Coordinate axes of the loaded scene.
    pub target_axes: CoordinateAxes,
    /// Length of one scene unit in meters.
    pub target_unit_meters: f64,
    /// How the axis/unit conversion is applied.
    pub space_conversion: SpaceConversion,
    /// How geometric transforms are applied.
    pub geometry_transform_handling: GeometryTransformHandling,
    /// How scale-compensating inheritance is applied.
    pub inherit_mode_handling: InheritModeHandling,
    /// Axes of cameras and lights after conversion (a camera looks down its
    /// -front axis, a light shines down its -front axis).
    pub target_camera_axes: CoordinateAxes,
    /// See [`LoadOptions::target_camera_axes`].
    pub target_light_axes: CoordinateAxes,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            target_axes: CoordinateAxes::RIGHT_HANDED_Y_UP,
            target_unit_meters: 1.0,
            space_conversion: SpaceConversion::ModifyGeometry,
            geometry_transform_handling: GeometryTransformHandling::ModifyGeometry,
            inherit_mode_handling: InheritModeHandling::Compensate,
            target_camera_axes: CoordinateAxes::RIGHT_HANDED_Y_UP,
            target_light_axes: CoordinateAxes::RIGHT_HANDED_Y_UP,
        }
    }
}

impl LoadOptions {
    fn check(&self) -> Result<(), Error> {
        let d = LoadOptions::default();
        if self.target_axes != d.target_axes {
            return Err(Error::UnsupportedOptions("target_axes"));
        }
        if self.target_unit_meters.to_bits() != d.target_unit_meters.to_bits() {
            return Err(Error::UnsupportedOptions("target_unit_meters"));
        }
        if self.target_camera_axes != d.target_camera_axes {
            return Err(Error::UnsupportedOptions("target_camera_axes"));
        }
        if self.target_light_axes != d.target_light_axes {
            return Err(Error::UnsupportedOptions("target_light_axes"));
        }
        Ok(())
    }
}

/// Load an FBX file (binary or ASCII, versions 7100 to 7700) with the
/// default [`Limits`].
///
/// # Errors
/// See [`Error`]. A failed load publishes no partial scene.
pub fn load(bytes: &[u8], options: &LoadOptions) -> Result<Scene, Error> {
    load_with_limits(bytes, options, &Limits::default())
}

/// [`load`] with explicit resource limits.
///
/// # Errors
/// See [`Error`].
pub fn load_with_limits(
    bytes: &[u8],
    options: &LoadOptions,
    limits: &Limits,
) -> Result<Scene, Error> {
    options.check()?;
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge {
            size: bytes.len(),
            limit: limits.max_input_bytes,
        });
    }
    let dump = limits.dump_limits();
    let canon = canon_scene::project_with(bytes, &dump, true)?;
    Scene::from_projection(canon, &dump)
}

/// A 3x4 affine matrix in column-major order: `[m00, m10, m20, m01, m11,
/// m21, m02, m12, m22, m03, m13, m23]`; the last column is the translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix(pub [f64; 12]);

impl Matrix {
    /// The identity matrix.
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);

    /// Column `i` (0..4); column 3 is the translation.
    pub fn column(&self, i: usize) -> [f64; 3] {
        let c = &self.0[i * 3..i * 3 + 3];
        [c[0], c[1], c[2]]
    }

    /// `self * p` for a point (translation applied).
    pub fn transform_point(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9],
            m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10],
            m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11],
        ]
    }
}

/// Translation, rotation (quaternion `[x, y, z, w]`) and scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Translation.
    pub translation: [f64; 3],
    /// Rotation quaternion `[x, y, z, w]`.
    pub rotation: [f64; 4],
    /// Scale.
    pub scale: [f64; 3],
}

/// Identity of a scene object.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ElementInfo {
    pub(crate) name: String,
    pub(crate) source_id: Option<i64>,
}

/// A per-corner vertex attribute: `values()` indexed through `indices()`
/// (one index per polygon corner), as in the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VertexAttribute<const N: usize> {
    values: Vec<f64>,
    indices: Vec<u32>,
}

impl<const N: usize> VertexAttribute<N> {
    pub(crate) fn from_flat(values: Vec<f64>, indices: Vec<u32>) -> Self {
        Self { values, indices }
    }

    /// The distinct values.
    pub fn values(&self) -> &[[f64; N]] {
        self.values.as_chunks::<N>().0
    }

    /// One index into [`VertexAttribute::values`] per polygon corner.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// The value of polygon corner `corner`.
    pub fn get(&self, corner: usize) -> Option<[f64; N]> {
        let i = *self.indices.get(corner)? as usize;
        self.values().get(i).copied()
    }
}

/// One polygon: `count` consecutive corners starting at `begin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    /// First corner index.
    pub begin: u32,
    /// Number of corners.
    pub count: u32,
}

/// A UV set with its optional tangent frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UvSet {
    pub(crate) name: String,
    pub(crate) uv: VertexAttribute<2>,
    pub(crate) tangent: Option<VertexAttribute<3>>,
    pub(crate) bitangent: Option<VertexAttribute<3>>,
}

impl UvSet {
    /// Set name as authored.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Texture coordinates.
    pub fn uv(&self) -> &VertexAttribute<2> {
        &self.uv
    }
    /// Tangents of this set, when the file has them.
    pub fn tangent(&self) -> Option<&VertexAttribute<3>> {
        self.tangent.as_ref()
    }
    /// Bitangents of this set, when the file has them.
    pub fn bitangent(&self) -> Option<&VertexAttribute<3>> {
        self.bitangent.as_ref()
    }
}

/// A vertex color set (RGBA).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColorSet {
    pub(crate) name: String,
    pub(crate) color: VertexAttribute<4>,
}

impl ColorSet {
    /// Set name as authored.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// RGBA colors.
    pub fn color(&self) -> &VertexAttribute<4> {
        &self.color
    }
}

/// The faces of a mesh that use one material slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshPart {
    pub(crate) index: u32,
    pub(crate) num_triangles: u64,
    pub(crate) face_indices: Vec<u32>,
}

impl MeshPart {
    /// Material slot (index into [`Mesh::materials`]).
    pub fn material_slot(&self) -> u32 {
        self.index
    }
    /// Number of triangles of the part's faces.
    pub fn num_triangles(&self) -> u64 {
        self.num_triangles
    }
    /// Faces of the part, ascending.
    pub fn face_indices(&self) -> &[u32] {
        &self.face_indices
    }
}

/// A polygon mesh with every attribute set of the file, converted to the
/// target space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub(crate) base: ElementInfo,
    pub(crate) num_vertices: usize,
    pub(crate) faces: Vec<Face>,
    pub(crate) positions: VertexAttribute<3>,
    pub(crate) normals: Option<VertexAttribute<3>>,
    pub(crate) uv_sets: Vec<UvSet>,
    pub(crate) color_sets: Vec<ColorSet>,
    pub(crate) face_material: Vec<u32>,
    pub(crate) materials: Vec<usize>,
    pub(crate) parts: Vec<MeshPart>,
    pub(crate) triangles: Vec<u32>,
    pub(crate) instances: Vec<usize>,
    pub(crate) reversed_winding: bool,
    pub(crate) generated_normals: bool,
    pub(crate) skin_deformers: Vec<usize>,
    pub(crate) blend_deformers: Vec<usize>,
}

impl Mesh {
    /// Skin deformers, indexed into [`Scene::skin_deformers`].
    pub fn skin_deformers(&self) -> &[usize] {
        &self.skin_deformers
    }
    /// Blend deformers, indexed into [`Scene::blend_deformers`].
    pub fn blend_deformers(&self) -> &[usize] {
        &self.blend_deformers
    }

    /// Object name.
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID.
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// Number of distinct control points (positions).
    pub fn num_vertices(&self) -> usize {
        self.num_vertices
    }
    /// Number of polygon corners.
    pub fn num_corners(&self) -> usize {
        self.positions.indices.len()
    }
    /// Polygons.
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }
    /// Positions.
    pub fn positions(&self) -> &VertexAttribute<3> {
        &self.positions
    }
    /// Normals: authored, or generated (see [`Mesh::generated_normals`]).
    pub fn normals(&self) -> Option<&VertexAttribute<3>> {
        self.normals.as_ref()
    }
    /// True when the file had no normals and they were generated.
    pub fn generated_normals(&self) -> bool {
        self.generated_normals
    }
    /// Every UV set, in file order.
    pub fn uv_sets(&self) -> &[UvSet] {
        &self.uv_sets
    }
    /// Every color set, in file order.
    pub fn color_sets(&self) -> &[ColorSet] {
        &self.color_sets
    }
    /// Per-face material slot (index into [`Mesh::materials`]); empty when
    /// the mesh has no material.
    pub fn face_material(&self) -> &[u32] {
        &self.face_material
    }
    /// Materials by slot, as indices into [`Scene::materials`].
    pub fn materials(&self) -> &[usize] {
        &self.materials
    }
    /// Faces grouped by material slot (one part when the mesh has at most
    /// one material).
    pub fn parts(&self) -> &[MeshPart] {
        &self.parts
    }
    /// Triangulated index buffer: three polygon-corner indices per triangle,
    /// faces in order, with ufbx's triangulation (ear clipping for n-gons).
    pub fn triangles(&self) -> &[u32] {
        &self.triangles
    }
    /// Nodes that instance this mesh (indices into [`Scene::nodes`]).
    pub fn instances(&self) -> &[usize] {
        &self.instances
    }
    /// True when the conversion mirrored the mesh and reversed each face's
    /// corner order to keep the winding.
    pub fn reversed_winding(&self) -> bool {
        self.reversed_winding
    }
}

/// A node of the hierarchy. Index 0 of [`Scene::nodes`] is the synthetic
/// root.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub(crate) base: ElementInfo,
    pub(crate) parent: Option<usize>,
    pub(crate) children: Vec<usize>,
    pub(crate) local: Transform,
    pub(crate) node_to_parent: Matrix,
    pub(crate) node_to_world: Matrix,
    pub(crate) geometry_to_node: Matrix,
    pub(crate) geometry_to_world: Matrix,
    pub(crate) visible: bool,
    pub(crate) mesh: Option<usize>,
    pub(crate) camera: Option<usize>,
    pub(crate) light: Option<usize>,
    pub(crate) materials: Vec<usize>,
    pub(crate) bone: Option<usize>,
    pub(crate) bind_pose: Option<usize>,
}

impl Node {
    /// Attached bone attribute, indexed into [`Scene::bones`].
    pub fn bone(&self) -> Option<usize> {
        self.bone
    }
    /// First bind pose containing this node, indexed into [`Scene::poses`].
    pub fn bind_pose(&self) -> Option<usize> {
        self.bind_pose
    }

    /// Object name (empty for the root).
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID (`None` for the root).
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// Parent node (`None` for the root).
    pub fn parent(&self) -> Option<usize> {
        self.parent
    }
    /// Child nodes, in connection order.
    pub fn children(&self) -> &[usize] {
        &self.children
    }
    /// Local transform relative to the parent.
    pub fn local_transform(&self) -> &Transform {
        &self.local
    }
    /// Local transform as a matrix.
    pub fn node_to_parent(&self) -> &Matrix {
        &self.node_to_parent
    }
    /// World transform.
    pub fn node_to_world(&self) -> &Matrix {
        &self.node_to_world
    }
    /// Geometric (attachment-only) transform; identity when it was baked
    /// into the mesh.
    pub fn geometry_to_node(&self) -> &Matrix {
        &self.geometry_to_node
    }
    /// World transform of attached geometry.
    pub fn geometry_to_world(&self) -> &Matrix {
        &self.geometry_to_world
    }
    /// Visibility property.
    pub fn visible(&self) -> bool {
        self.visible
    }
    /// Attached mesh (index into [`Scene::meshes`]).
    pub fn mesh(&self) -> Option<usize> {
        self.mesh
    }
    /// Attached camera (index into [`Scene::cameras`]).
    pub fn camera(&self) -> Option<usize> {
        self.camera
    }
    /// Attached light (index into [`Scene::lights`]).
    pub fn light(&self) -> Option<usize> {
        self.light
    }
    /// Materials connected to the node (indices into [`Scene::materials`]).
    pub fn materials(&self) -> &[usize] {
        &self.materials
    }
}

/// One material map: a value and an optional texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialMap {
    /// Value (`value_components` of the four are meaningful).
    pub value: [f64; 4],
    /// Integer value, for integer-valued maps.
    pub value_int: i64,
    /// Number of meaningful components of `value` (0 when unset).
    pub value_components: u64,
    /// Texture (index into [`Scene::textures`]).
    pub texture: Option<usize>,
    /// True when the file sets a value.
    pub has_value: bool,
    /// False when the file disables the texture.
    pub texture_enabled: bool,
    /// True when the owning feature is disabled.
    pub feature_disabled: bool,
}

/// A material with FBX maps and derived PBR maps.
///
/// PBR mappings cover FBX Lambert/Phong (also the unknown-shader fallback),
/// 3ds Max Physical Material, 3ds Max PBR Metal/Rough and Spec/Gloss, glTF
/// Material, and OpenPBR Material identified by their 3ds Max class IDs.
/// Shader-element bindings (including Arnold, OSL and ShaderFX/Stingray)
/// and Blender-specific Phong mapping are not implemented.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub(crate) base: ElementInfo,
    pub(crate) shading_model: String,
    pub(crate) fbx: Vec<(&'static str, MaterialMap)>,
    pub(crate) pbr: Vec<(&'static str, MaterialMap)>,
    pub(crate) features: Vec<(&'static str, MaterialFeature)>,
    pub(crate) properties: Vec<SourceProperty>,
    pub(crate) textures: Vec<MaterialTexture>,
}

/// A computed material feature and whether it was explicitly authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MaterialFeature {
    /// Computed enabled state.
    pub enabled: bool,
    /// True when explicitly set by the source shader.
    pub is_explicit: bool,
}
/// A retained source property, including the selected source template defaults.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SourceProperty {
    /// Byte-exact property name.
    pub name: Vec<u8>,
    /// ufbx property type discriminant.
    pub property_type: u8,
    /// ufbx property flags.
    pub flags: u32,
    /// Integer view.
    pub value_int: i64,
    /// Numeric components.
    pub value_real: [f64; 4],
    /// Byte-exact string view.
    pub value_str: Vec<u8>,
    /// Blob view.
    pub value_blob: Vec<u8>,
}
/// One material texture attachment, including unmapped source properties.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MaterialTexture {
    /// Byte-exact material property name.
    pub material_prop: Vec<u8>,
    /// Byte-exact shader property name.
    pub shader_prop: Vec<u8>,
    /// Index into [`Scene::textures`].
    pub texture: usize,
}
/// One ordered child of a layered texture.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TextureLayer {
    /// Index into [`Scene::textures`].
    pub texture: usize,
    /// ufbx blend mode discriminant.
    pub blend_mode: i64,
    /// Layer alpha.
    pub alpha: f64,
}
impl Material {
    /// All computed feature flags, in ufbx field order.
    pub fn features(&self) -> impl Iterator<Item = (&'static str, &MaterialFeature)> {
        self.features.iter().map(|(n, f)| (*n, f))
    }
    /// A feature by ufbx name, such as `double_sided` or `unlit`.
    pub fn feature(&self, name: &str) -> Option<&MaterialFeature> {
        self.features
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
    }
    /// Source properties in byte-name order; authored values override templates.
    pub fn source_properties(&self) -> &[SourceProperty] {
        &self.properties
    }
    /// Look up an authored property, falling back to the source template.
    pub fn source_property(&self, name: &[u8]) -> Option<&SourceProperty> {
        self.properties
            .binary_search_by(|p| p.name.as_slice().cmp(name))
            .ok()
            .map(|i| &self.properties[i])
    }
    /// Every attachment, including properties without a mapped FBX/PBR slot.
    pub fn textures(&self) -> &[MaterialTexture] {
        &self.textures
    }
    /// Object name.
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID.
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// `ShadingModel` as authored (`"Phong"`, `"Lambert"`, ...).
    pub fn shading_model(&self) -> &str {
        &self.shading_model
    }
    /// FBX maps by ufbx name (`diffuse_color`, `normal_map`, ...).
    pub fn fbx_maps(&self) -> impl Iterator<Item = (&'static str, &MaterialMap)> {
        self.fbx.iter().map(|(n, m)| (*n, m))
    }
    /// PBR maps by ufbx name (`base_color`, `roughness`, `metalness`, ...).
    pub fn pbr_maps(&self) -> impl Iterator<Item = (&'static str, &MaterialMap)> {
        self.pbr.iter().map(|(n, m)| (*n, m))
    }
    /// One FBX map by name.
    pub fn fbx_map(&self, name: &str) -> Option<&MaterialMap> {
        self.fbx.iter().find(|(n, _)| *n == name).map(|(_, m)| m)
    }
    /// One PBR map by name.
    pub fn pbr_map(&self, name: &str) -> Option<&MaterialMap> {
        self.pbr.iter().find(|(n, _)| *n == name).map(|(_, m)| m)
    }
}

/// A texture: file paths as written and the embedded bytes, if any. Paths
/// are never resolved against the file system.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub(crate) base: ElementInfo,
    pub(crate) layered: bool,
    pub(crate) layers: Vec<TextureLayer>,
    pub(crate) file_textures: Vec<usize>,
    pub(crate) filename: String,
    pub(crate) relative_filename: String,
    pub(crate) absolute_filename: String,
    pub(crate) content: Vec<u8>,
    pub(crate) uv_set: String,
    pub(crate) wrap: [i64; 2],
}

impl Texture {
    /// Ordered child layers with blend modes and alpha.
    pub fn layers(&self) -> &[TextureLayer] {
        &self.layers
    }
    /// Flattened unique file texture indices, in ufbx traversal order.
    pub fn file_textures(&self) -> &[usize] {
        &self.file_textures
    }

    /// Object name.
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID.
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// True for a layered texture (no file of its own).
    pub fn is_layered(&self) -> bool {
        self.layered
    }
    /// Best path: the relative path resolved against the file's own
    /// directory as written, else the absolute path.
    pub fn filename(&self) -> &str {
        &self.filename
    }
    /// Path relative to the FBX file, as written.
    pub fn relative_filename(&self) -> &str {
        &self.relative_filename
    }
    /// Absolute path, as written.
    pub fn absolute_filename(&self) -> &str {
        &self.absolute_filename
    }
    /// Embedded file bytes (empty when the image is external).
    pub fn content(&self) -> &[u8] {
        &self.content
    }
    /// UV set name the texture samples (empty for the default set).
    pub fn uv_set(&self) -> &str {
        &self.uv_set
    }
    /// Wrap modes U and V: 0 repeat, 1 clamp.
    pub fn wrap(&self) -> [i64; 2] {
        self.wrap
    }
}

/// A camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub(crate) base: ElementInfo,
    pub(crate) projection_mode: i64,
    pub(crate) field_of_view_deg: [f64; 2],
    pub(crate) near_plane: f64,
    pub(crate) far_plane: f64,
    pub(crate) aspect_ratio: f64,
    pub(crate) orthographic_size: [f64; 2],
    pub(crate) focal_length_mm: f64,
}

impl Camera {
    /// Object name.
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID.
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// True for an orthographic camera.
    pub fn is_orthographic(&self) -> bool {
        self.projection_mode == 1
    }
    /// Horizontal and vertical field of view in degrees.
    pub fn field_of_view_deg(&self) -> [f64; 2] {
        self.field_of_view_deg
    }
    /// Near clip plane distance.
    pub fn near_plane(&self) -> f64 {
        self.near_plane
    }
    /// Far clip plane distance.
    pub fn far_plane(&self) -> f64 {
        self.far_plane
    }
    /// Width over height.
    pub fn aspect_ratio(&self) -> f64 {
        self.aspect_ratio
    }
    /// Orthographic width and height.
    pub fn orthographic_size(&self) -> [f64; 2] {
        self.orthographic_size
    }
    /// Focal length in millimeters.
    pub fn focal_length_mm(&self) -> f64 {
        self.focal_length_mm
    }
}

/// Light kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LightKind {
    /// Point light.
    Point,
    /// Directional light.
    Directional,
    /// Spot light.
    Spot,
    /// Area light.
    Area,
    /// Volume light.
    Volume,
}

/// A light.
#[derive(Debug, Clone, PartialEq)]
pub struct Light {
    pub(crate) base: ElementInfo,
    pub(crate) kind: LightKind,
    pub(crate) color: [f64; 3],
    pub(crate) intensity: f64,
    pub(crate) local_direction: [f64; 3],
    pub(crate) decay: i64,
    pub(crate) inner_angle: f64,
    pub(crate) outer_angle: f64,
    pub(crate) cast_light: bool,
    pub(crate) cast_shadows: bool,
}

impl Light {
    /// Object name.
    pub fn name(&self) -> &str {
        &self.base.name
    }
    /// FBX object ID.
    pub fn source_id(&self) -> Option<i64> {
        self.base.source_id
    }
    /// Kind.
    pub fn kind(&self) -> LightKind {
        self.kind
    }
    /// Linear RGB color.
    pub fn color(&self) -> [f64; 3] {
        self.color
    }
    /// Intensity (the FBX value divided by 100).
    pub fn intensity(&self) -> f64 {
        self.intensity
    }
    /// Direction the light points in node space.
    pub fn local_direction(&self) -> [f64; 3] {
        self.local_direction
    }
    /// Decay: 0 none, 1 linear, 2 quadratic, 3 cubic.
    pub fn decay(&self) -> i64 {
        self.decay
    }
    /// Spot inner cone angle in degrees.
    pub fn inner_angle(&self) -> f64 {
        self.inner_angle
    }
    /// Spot outer cone angle in degrees.
    pub fn outer_angle(&self) -> f64 {
        self.outer_angle
    }
    /// True when the light emits.
    pub fn cast_light(&self) -> bool {
        self.cast_light
    }
    /// True when the light casts shadows.
    pub fn cast_shadows(&self) -> bool {
        self.cast_shadows
    }
}

/// Scene-wide settings and file metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSettings {
    pub(crate) axes: CoordinateAxes,
    pub(crate) unit_meters: f64,
    pub(crate) original_axis_up: Option<Axis>,
    pub(crate) original_unit_meters: f64,
    pub(crate) frames_per_second: f64,
    pub(crate) ambient_color: [f64; 3],
    pub(crate) version: u32,
    pub(crate) ascii: bool,
    pub(crate) creator: String,
    pub(crate) original_application: [String; 3],
    pub(crate) original_file_path: String,
}

impl SceneSettings {
    /// Axes of the loaded scene (the target axes).
    pub fn axes(&self) -> CoordinateAxes {
        self.axes
    }
    /// Meters per scene unit after conversion.
    pub fn unit_meters(&self) -> f64 {
        self.unit_meters
    }
    /// Up axis of the authoring application, when recorded.
    pub fn original_axis_up(&self) -> Option<Axis> {
        self.original_axis_up
    }
    /// Meters per unit of the authoring application.
    pub fn original_unit_meters(&self) -> f64 {
        self.original_unit_meters
    }
    /// Frame rate.
    pub fn frames_per_second(&self) -> f64 {
        self.frames_per_second
    }
    /// Scene ambient color.
    pub fn ambient_color(&self) -> [f64; 3] {
        self.ambient_color
    }
    /// FBX version (7100 to 7700).
    pub fn version(&self) -> u32 {
        self.version
    }
    /// True for an ASCII file.
    pub fn is_ascii(&self) -> bool {
        self.ascii
    }
    /// `Creator` string of the file.
    pub fn creator(&self) -> &str {
        &self.creator
    }
    /// Original application vendor, name and version.
    pub fn original_application(&self) -> [&str; 3] {
        let [a, b, c] = &self.original_application;
        [a, b, c]
    }
    /// Path the file was saved to, as recorded in the file.
    pub fn original_file_path(&self) -> &str {
        &self.original_file_path
    }
}

/// Content the static loader does not convert, reported per object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unsupported {
    /// What kind of content.
    pub kind: UnsupportedKind,
    /// FBX object ID (the owning geometry for legacy shapes).
    pub source_id: i64,
}

/// A loaded scene. Objects refer to each other by index into the slices.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    animation: crate::animation::Animation,
    nodes: Vec<Node>,
    meshes: Vec<Mesh>,
    materials: Vec<Material>,
    textures: Vec<Texture>,
    cameras: Vec<Camera>,
    lights: Vec<Light>,
    bones: Vec<crate::Bone>,
    skin_deformers: Vec<crate::SkinDeformer>,
    skin_clusters: Vec<crate::SkinCluster>,
    blend_deformers: Vec<crate::BlendDeformer>,
    blend_channels: Vec<crate::BlendChannel>,
    blend_shapes: Vec<crate::BlendShape>,
    poses: Vec<crate::Pose>,
    settings: SceneSettings,
    unsupported: Vec<Unsupported>,
    warnings: Vec<String>,
}

impl Scene {
    /// Bake a stack into independently reduced linear transform/property channels.
    /// `None` selects the default stack. Candidate keys and allocation/work
    /// are bounded by `options`; see `docs/VERIFICATION.md` for compatibility.
    pub fn bake_animation(
        &self,
        stack: Option<usize>,
        options: &crate::BakeOptions,
    ) -> Result<crate::BakedAnimation, Error> {
        crate::bake::bake(&self.animation, stack, options)
    }

    /// Animation stacks in source identity order.
    pub fn animation_stacks(&self) -> &[crate::AnimationStack] {
        &self.animation.stacks
    }
    /// Index of the default take, chosen in source object order.
    pub fn default_animation_stack(&self) -> Option<usize> {
        self.animation.default_stack
    }
    /// Animation layers; stack layer indices refer to this slice.
    pub fn animation_layers(&self) -> &[crate::AnimationLayer] {
        &self.animation.layers
    }
    /// Animation values; bindings refer to this slice.
    pub fn animation_values(&self) -> &[crate::AnimationValue] {
        &self.animation.values
    }
    /// Authored curves with reconstructed tangent handles.
    pub fn animation_curves(&self) -> &[crate::AnimationCurve] {
        &self.animation.curves
    }
    /// Evaluate a full pose in seconds. `None` selects the default stack in source object order.
    pub fn evaluate(
        &self,
        stack: Option<usize>,
        time: f64,
    ) -> Result<crate::EvaluatedScene, Error> {
        self.animation.evaluate(stack, time)
    }
    /// Evaluate one local node transform. Node indices match [`Self::nodes`].
    pub fn evaluate_transform(
        &self,
        stack: Option<usize>,
        node: usize,
        time: f64,
    ) -> Result<Transform, Error> {
        self.evaluate(stack, time)?
            .nodes
            .get(node)
            .map(|n| n.local_transform)
            .ok_or(Error::InvalidData("animation: node index out of range"))
    }
    /// Evaluate a binding from this scene, including properties on generated helpers.
    pub fn evaluate_binding(
        &self,
        stack: Option<usize>,
        binding: &crate::AnimationBinding,
        time: f64,
    ) -> Result<crate::EvaluatedProperty, Error> {
        if !time.is_finite() {
            return Err(Error::InvalidData("animation: nonfinite evaluation time"));
        }
        if !self
            .animation
            .layers
            .iter()
            .any(|l| l.bindings.contains(binding))
        {
            return Err(Error::InvalidData(
                "animation: binding does not belong to scene",
            ));
        }
        self.animation.property(
            self.animation.stack_layers(stack)?,
            binding.element,
            &binding.property,
            time,
        )
    }
    /// Evaluate any object's property by its source ID and byte-exact property name.
    pub fn evaluate_property(
        &self,
        stack: Option<usize>,
        source_id: i64,
        name: &[u8],
        time: f64,
    ) -> Result<crate::EvaluatedProperty, Error> {
        if !time.is_finite() {
            return Err(Error::InvalidData("animation: nonfinite evaluation time"));
        }
        let layers = self.animation.stack_layers(stack)?;
        let i = self
            .animation
            .ids
            .iter()
            .position(|id| *id == Some(source_id))
            .ok_or(Error::InvalidData("animation: source ID not found"))?;
        self.animation.property(layers, i, name, time)
    }
    /// Stored bones; references use indices into the corresponding scene slice.
    pub fn bones(&self) -> &[crate::Bone] {
        &self.bones
    }
    /// Stored skin deformers; references use indices into the corresponding scene slice.
    pub fn skin_deformers(&self) -> &[crate::SkinDeformer] {
        &self.skin_deformers
    }
    /// Stored skin clusters; references use indices into the corresponding scene slice.
    pub fn skin_clusters(&self) -> &[crate::SkinCluster] {
        &self.skin_clusters
    }
    /// Stored blend deformers; references use indices into the corresponding scene slice.
    pub fn blend_deformers(&self) -> &[crate::BlendDeformer] {
        &self.blend_deformers
    }
    /// Stored blend channels; references use indices into the corresponding scene slice.
    pub fn blend_channels(&self) -> &[crate::BlendChannel] {
        &self.blend_channels
    }
    /// Stored blend shapes; references use indices into the corresponding scene slice.
    pub fn blend_shapes(&self) -> &[crate::BlendShape] {
        &self.blend_shapes
    }
    /// Stored poses; references use indices into the corresponding scene slice.
    pub fn poses(&self) -> &[crate::Pose] {
        &self.poses
    }

    /// Conversion warnings: content that was read but is not converted
    /// (unported shader types, shader textures) or a mesh that failed to
    /// convert and is left empty. The text is for people, not for matching.
    /// The library never prints them.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    /// Nodes; index 0 is the root.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    /// The root node.
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }
    /// Meshes.
    pub fn meshes(&self) -> &[Mesh] {
        &self.meshes
    }
    /// Materials.
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }
    /// Textures.
    pub fn textures(&self) -> &[Texture] {
        &self.textures
    }
    /// Cameras.
    pub fn cameras(&self) -> &[Camera] {
        &self.cameras
    }
    /// Lights.
    pub fn lights(&self) -> &[Light] {
        &self.lights
    }
    /// Settings and metadata.
    pub fn settings(&self) -> &SceneSettings {
        &self.settings
    }
    /// Objects present in the file but not converted (surfaces and
    /// surfaces, caches, constraints, audio).
    pub fn unsupported(&self) -> &[Unsupported] {
        &self.unsupported
    }

    fn from_projection(mut c: CanonScene<'_>, limits: &DumpLimits) -> Result<Self, Error> {
        use ElementType as T;
        let budget = crate::api_alloc::Budget::new(limits.max_allocation);
        let n = c.elements.len();
        // Element index -> index within its kind.
        let mut slot = budget.filled(n, usize::MAX)?;
        let mut counts = [0usize; 13];
        for (i, e) in c.elements.iter().enumerate() {
            let k = match e.ty {
                T::Node => 0,
                T::Mesh => 1,
                T::Material => 2,
                T::Texture => 3,
                T::Camera => 4,
                T::Light => 5,
                T::Bone => 6,
                T::SkinDeformer => 7,
                T::SkinCluster => 8,
                T::BlendDeformer => 9,
                T::BlendChannel => 10,
                T::BlendShape => 11,
                T::Pose => 12,
                _ => continue,
            };
            slot[i] = counts[k];
            counts[k] += 1;
        }
        let r = |k: usize| slot.get(k).copied().filter(|&s| s != usize::MAX);
        let refs = |l: &[usize]| budget.collect(l.iter().filter_map(|&k| r(k)));
        let info = |e: &canon_scene::Element| -> Result<ElementInfo, Error> {
            Ok(ElementInfo {
                name: budget.text(&e.name)?,
                source_id: e.source_id,
            })
        };
        let mut t = std::mem::take(&mut c.typed);
        // Typed results follow ufbx's typed order; slices follow element order.
        t.materials.sort_unstable_by_key(|(i, _)| *i);
        t.textures.sort_unstable_by_key(|(i, _)| *i);
        let ty_of = |k: usize| c.elements[k].ty;

        let mut nodes = Vec::new();
        budget.reserve(&mut nodes, counts[0])?;
        for (i, e) in c.elements.iter().enumerate() {
            if e.ty != T::Node {
                continue;
            }
            let Some(Some(state)) = c.node_states.get(i) else {
                return Err(Error::InvalidData("node without transform"));
            };
            let first = t.attribs.get(i).and_then(|a| a.first().copied());
            let attrib = |want: ElementType| first.filter(|&a| ty_of(a) == want).and_then(r);
            nodes.push(Node {
                base: info(e)?,
                parent: t.parent.get(i).copied().flatten().and_then(r),
                children: refs(t.children.get(i).map_or(&[], Vec::as_slice))?,
                local: Transform {
                    translation: state.local.translation,
                    rotation: state.local.rotation,
                    scale: state.local.scale,
                },
                node_to_parent: Matrix(state.node_to_parent),
                node_to_world: Matrix(state.node_to_world),
                geometry_to_node: Matrix(state.geometry_to_node),
                geometry_to_world: Matrix(state.geometry_to_world),
                visible: t.visible.get(i).copied().unwrap_or(true),
                mesh: attrib(T::Mesh),
                camera: attrib(T::Camera),
                light: attrib(T::Light),
                bone: attrib(T::Bone),
                bind_pose: match e.obj.get("bind_pose") {
                    Some(V::Ref(Some(k))) => r(*k),
                    _ => None,
                },
                materials: refs(t.node_materials.get(i).map_or(&[], Vec::as_slice))?,
            });
        }
        let maps = |names: &[&'static str], maps: Vec<crate::scene_material::Map>| {
            budget.collect(names.iter().zip(maps).map(|(&name, m)| {
                (
                    name,
                    MaterialMap {
                        value: m.value,
                        value_int: m.int,
                        value_components: m.components,
                        texture: m.texture.and_then(r),
                        has_value: m.has_value,
                        texture_enabled: m.texture_enabled,
                        feature_disabled: m.feature_disabled,
                    },
                )
            }))
        };
        let text = |b| budget.owned_text(b);
        use crate::scene_material::{FBX_MAPS, PBR_MAPS};
        let materials = budget.try_collect(t.materials.into_iter().map(|(i, m)| {
            Ok(Material {
                base: info(&c.elements[i])?,
                features: budget.collect(
                    crate::scene_material::FEATURES.iter().zip(m.features).map(
                        |(&n, (enabled, is_explicit))| {
                            (
                                n,
                                MaterialFeature {
                                    enabled,
                                    is_explicit,
                                },
                            )
                        },
                    ),
                )?,
                properties: m.properties,
                textures: budget.try_collect(m.attachments.into_iter().map(|a| {
                    Ok(MaterialTexture {
                        material_prop: a.material_prop,
                        shader_prop: a.shader_prop,
                        texture: r(a.texture)
                            .ok_or(Error::ResourceLimit("material texture index"))?,
                    })
                }))?,
                shading_model: text(m.shading_model)?,
                fbx: maps(&FBX_MAPS, m.fbx)?,
                pbr: maps(&PBR_MAPS, m.pbr)?,
            })
        }))?;
        let textures = budget.try_collect(t.textures.into_iter().map(|(i, x)| {
            Ok(Texture {
                base: info(&c.elements[i])?,
                layered: x.layered,
                layers: budget.try_collect(x.layers.into_iter().map(
                    |(t, blend_mode, alpha)| {
                        Ok(TextureLayer {
                            texture: r(t).ok_or(Error::ResourceLimit("texture layer index"))?,
                            blend_mode,
                            alpha,
                        })
                    },
                ))?,
                file_textures: refs(&x.file_textures)?,
                filename: text(x.filename)?,
                relative_filename: text(x.relative_filename)?,
                absolute_filename: text(x.absolute_filename)?,
                content: x.content,
                uv_set: text(x.uv_set)?,
                wrap: x.wrap,
            })
        }))?;
        let cameras = budget.try_collect(t.cameras.iter().map(|(i, x)| {
            Ok(Camera {
                base: info(&c.elements[*i])?,
                projection_mode: x.projection_mode,
                field_of_view_deg: x.field_of_view_deg,
                near_plane: x.near_plane,
                far_plane: x.far_plane,
                aspect_ratio: x.aspect_ratio,
                orthographic_size: x.orthographic_size,
                focal_length_mm: x.focal_length_mm,
            })
        }))?;
        let lights = budget.try_collect(t.lights.iter().map(|(i, x)| {
            Ok(Light {
                base: info(&c.elements[*i])?,
                kind: match x.ty {
                    1 => LightKind::Directional,
                    2 => LightKind::Spot,
                    3 => LightKind::Area,
                    4 => LightKind::Volume,
                    _ => LightKind::Point,
                },
                color: x.color,
                intensity: x.intensity,
                local_direction: x.local_direction,
                decay: x.decay,
                inner_angle: x.inner_angle,
                outer_angle: x.outer_angle,
                cast_light: x.cast_light,
                cast_shadows: x.cast_shadows,
            })
        }))?;
        let mut mesh_instances =
            budget.try_collect(c.elements.iter().map(|e| refs(&e.instances)))?;

        // Meshes: typed conversion results plus ufbx triangulation of the
        // source faces (corner order is shared; reversed winding mirrors it).
        let index = scene::index(&c.doc, &limits.index).map_err(scene_error)?;
        let sources = scene_mesh::extract(&c.doc, &index, &scene_mesh::Limits::default())
            .map_err(scene_error)?;
        let mut sources_by_node = budget.collect(sources.meshes.iter().map(|s| (s.node, s)))?;
        sources_by_node.sort_unstable_by_key(|s| s.0);
        let tl = TriangulationLimits::default();
        let mut converted = t.meshes;
        converted.sort_unstable_by_key(|(i, _, _)| *i);
        let mut meshes = Vec::new();
        budget.reserve(&mut meshes, counts[1])?;
        let mut done = budget.filled(n, false)?;
        for (i, m, mesh_materials) in converted {
            let e = &c.elements[i];
            let source = e.node.and_then(|node| {
                sources_by_node
                    .binary_search_by_key(&node, |s| s.0)
                    .ok()
                    .map(|k| sources_by_node[k].1)
            });
            let triangles = match source {
                Some(s) => triangulate(s, &m, &tl, &budget)?,
                None => Vec::new(),
            };
            let materials = refs(&mesh_materials)?;
            let instances = std::mem::take(&mut mesh_instances[i]);
            let base = info(e)?;
            done[i] = true;
            meshes.push((
                i,
                crate::scene_geometry::public_mesh(
                    m, base, materials, instances, triangles, &budget,
                )?,
            ));
        }
        // Meshes that failed conversion stay as empty meshes so indices hold.
        for (i, e) in c.elements.iter().enumerate() {
            if e.ty == T::Mesh && !done[i] {
                let mut m = Mesh {
                    base: info(e)?,
                    ..Mesh::default()
                };
                m.instances = std::mem::take(&mut mesh_instances[i]);
                meshes.push((i, m));
            }
        }
        meshes.sort_unstable_by_key(|(i, _)| *i);
        let mut meshes = budget.collect(meshes.into_iter().map(|(_, m)| m))?;

        let st = &c.settings;
        let bone_refs = |e: &canon_scene::Element, want: T| {
            budget.collect(
                e.dst_conns
                    .iter()
                    .filter(|(k, p)| p.is_empty() && c.elements[*k].ty == want)
                    .filter_map(|(k, _)| r(*k)),
            )
        };
        if !c.deformers.skins.is_empty() || !c.deformers.blends.is_empty() {
            let mut seen = budget.filled(n, 0usize)?;
            let mut stamp = 0usize;
            for (mesh, e) in meshes
                .iter_mut()
                .zip(c.elements.iter().filter(|e| e.ty == T::Mesh))
            {
                let mut fetch = |ty| -> Result<Vec<usize>, Error> {
                    stamp += 1;
                    let mut out = bone_refs(e, ty)?;
                    for &k in &out {
                        seen[k] = stamp;
                    }
                    if let Some(&node) = e.instances.first() {
                        for k in bone_refs(&c.elements[node], ty)? {
                            if seen[k] != stamp {
                                seen[k] = stamp;
                                budget.push(&mut out, k)?;
                            }
                        }
                    }
                    Ok(out)
                };
                mesh.skin_deformers = fetch(T::SkinDeformer)?;
                mesh.blend_deformers = fetch(T::BlendDeformer)?;
            }
        }
        let bones = budget.try_collect(c.elements.iter().filter(|e| e.ty == T::Bone).map(|e| {
            Ok(crate::Bone {
                name: budget.text(&e.name)?,
                source_id: e.source_id,
                instances: refs(&e.instances)?,
                radius: real(e.obj.get("radius")),
                relative_length: real(e.obj.get("relative_length")),
                is_root: matches!(e.obj.get("is_root"), Some(V::Bool(true))),
            })
        }))?;
        let mut d = std::mem::take(&mut c.deformers);
        for (_, skin) in &mut d.skins {
            skin.clusters = refs(&skin.clusters)?;
        }
        for (_, cluster) in &mut d.clusters {
            cluster.bone_node = cluster.bone_node.and_then(r);
        }
        for (_, blend) in &mut d.blends {
            blend.channels = refs(&blend.channels)?;
        }
        for (_, channel) in &mut d.channels {
            channel.target_shape = channel.target_shape.and_then(r);
            for key in &mut channel.keyframes {
                key.shape = r(key.shape).ok_or(Error::InvalidData("blend target index"))?;
            }
        }
        for (_, pose) in &mut d.poses {
            for b in &mut pose.bone_poses {
                b.bone_node = r(b.bone_node).ok_or(Error::InvalidData("pose node index"))?;
            }
        }
        let skin_deformers = budget.collect(d.skins.into_iter().map(|(_, v)| v))?;
        let skin_clusters = budget.collect(d.clusters.into_iter().map(|(_, v)| v))?;
        let blend_deformers = budget.collect(d.blends.into_iter().map(|(_, v)| v))?;
        let blend_channels = budget.collect(d.channels.into_iter().map(|(_, v)| v))?;
        let blend_shapes = budget.collect(d.shapes.into_iter().map(|(_, v)| v))?;
        let poses = budget.collect(d.poses.into_iter().map(|(_, v)| v))?;
        let md = &c.metadata;
        let axes = match st.get("axes") {
            Some(V::Obj(a)) => CoordinateAxes {
                right: Axis::from_ufbx(int(a.get("right"))).unwrap_or(Axis::PositiveX),
                up: Axis::from_ufbx(int(a.get("up"))).unwrap_or(Axis::PositiveY),
                front: Axis::from_ufbx(int(a.get("front"))).unwrap_or(Axis::PositiveZ),
            },
            _ => CoordinateAxes::RIGHT_HANDED_Y_UP,
        };
        let app = |k: &str| match md.get("original_application") {
            Some(V::Obj(a)) => projection_string(&budget, a.get(k)),
            _ => Ok(String::new()),
        };
        let settings = SceneSettings {
            axes,
            unit_meters: real(st.get("unit_meters")),
            original_axis_up: Axis::from_ufbx(int(st.get("original_axis_up"))),
            original_unit_meters: real(st.get("original_unit_meters")),
            frames_per_second: real(st.get("frames_per_second")),
            ambient_color: arr(st.get("ambient_color"), [0.0; 3]),
            version: c.doc.version,
            ascii: c.ascii,
            creator: projection_string(&budget, md.get("creator"))?,
            original_application: [app("vendor")?, app("name")?, app("version")?],
            original_file_path: budget.text(&c.original_file_path.0)?,
        };
        let unsupported = budget.collect(
            c.unsupported
                .iter()
                .map(|&(kind, source_id)| Unsupported { kind, source_id }),
        )?;
        Ok(Scene {
            animation: std::mem::take(&mut c.animation),
            nodes,
            meshes,
            materials,
            textures,
            cameras,
            lights,
            bones,
            skin_deformers,
            skin_clusters,
            blend_deformers,
            blend_channels,
            blend_shapes,
            poses,
            settings,
            unsupported,
            warnings: std::mem::take(&mut c.warnings),
        })
    }
}

fn triangulate(
    source: &scene_mesh::Mesh<'_>,
    m: &crate::scene_geometry::Mesh,
    limits: &TriangulationLimits,
    budget: &crate::api_alloc::Budget,
) -> Result<Vec<u32>, Error> {
    let bad = scene_error;
    let faces = source.faces().map_err(bad)?;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for (face, range) in faces.enumerate() {
        let range = range.map_err(bad)?;
        let (begin, count) = (range.start, range.len());
        if m.face(face)
            != Some((
                u32::try_from(begin)
                    .map_err(|_| Error::InvalidData("triangulation: index overflow"))?,
                u32::try_from(count)
                    .map_err(|_| Error::InvalidData("triangulation: count overflow"))?,
            ))
        {
            return Err(Error::InvalidData("triangulation: face layout mismatch"));
        }
        buf.clear();
        let corners = count
            .saturating_sub(2)
            .checked_mul(3)
            .ok_or(Error::ResourceLimit("triangulation corners"))?;
        budget.reserve(&mut buf, corners)?;
        buf.resize(corners, 0usize);
        budget.reserve(&mut out, corners)?;
        let written = source
            .triangulate_with_limits(begin, count, &mut buf, limits)
            .map_err(bad)?;
        for &corner in &buf[..written * 3] {
            // `flip` keeps a face's first corner and reverses the rest.
            let c = if m.reversed_winding() && corner != begin {
                begin + count - (corner - begin)
            } else {
                corner
            };
            out.push(
                u32::try_from(c)
                    .map_err(|_| Error::InvalidData("triangulation: corner overflow"))?,
            );
        }
    }
    Ok(out)
}

fn projection_string(b: &crate::api_alloc::Budget, v: Option<&V>) -> Result<String, Error> {
    match v {
        Some(V::Bytes(bytes)) => b.text(bytes),
        _ => Ok(String::new()),
    }
}

pub(crate) fn property_error(e: crate::scene_property::Error) -> Error {
    match e {
        crate::scene_property::Error::AllocationFailed => Error::Allocation,
        crate::scene_property::Error::LimitExceeded => {
            Error::ResourceLimit("property allocation limit")
        }
        _ => match crate::api_alloc::Budget::new(usize::MAX)
            .format(format_args!("properties: {e:?}"))
        {
            Ok(s) => Error::Convert(s),
            Err(e) => e,
        },
    }
}
pub(crate) fn scene_error(e: crate::scene::Error) -> Error {
    match e {
        crate::scene::Error::AllocationFailed => Error::Allocation,
        crate::scene::Error::LimitExceeded(s) => Error::ResourceLimit(s),
        _ => match crate::api_alloc::Budget::new(usize::MAX).format(format_args!("{e:?}")) {
            Ok(message) => Error::Convert(message),
            Err(error) => error,
        },
    }
}

fn real(v: Option<&V>) -> f64 {
    match v {
        Some(V::F(x)) => *x,
        Some(V::F32(x)) => f64::from(*x),
        Some(V::I(x)) => *x as f64,
        Some(V::U(x)) => *x as f64,
        _ => 0.0,
    }
}

fn int(v: Option<&V>) -> i64 {
    match v {
        Some(V::I(x)) => *x,
        Some(V::U(x)) => i64::try_from(*x).unwrap_or(i64::MAX),
        _ => 0,
    }
}

fn arr<const N: usize>(v: Option<&V>, default: [f64; N]) -> [f64; N] {
    match v {
        Some(V::Vec(x)) if x.len() >= N => {
            let mut out = default;
            out.copy_from_slice(&x[..N]);
            out
        }
        _ => default,
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    const FIXTURE: &[u8] = br#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Geometry: 1, "Geometry::Tri", "Mesh" {
  Vertices: *9 { a: 0,0,0,1,0,0,0,1,0 }
  PolygonVertexIndex: *3 { a: 0,1,-3 }
  LayerElementUV: 0 {
   Name: "UV0"
   MappingInformationType: "ByPolygonVertex"
   ReferenceInformationType: "Direct"
   UV: *6 { a: 0,0,1,0,0,1 }
  }
  LayerElementColor: 0 {
   Name: "Color0"
   MappingInformationType: "ByPolygonVertex"
   ReferenceInformationType: "Direct"
   Colors: *12 { a: 1,0,0,1,0,1,0,1,0,0,1,1 }
  }
  Layer: 0 {
   LayerElement: { Type: "LayerElementUV"
    TypedIndex: 0 }
   LayerElement: { Type: "LayerElementColor"
    TypedIndex: 0 }
  }
 }
 Model: 2, "Model::Tri", "Mesh" { }
 Material: 3, "Material::Mat", "" {
  ShadingModel: "Phong"
  Properties70: { P: "Opacity", "Number", "", "", 0.25 }
 }
 Texture: 4, "Texture::File", "" { FileName: "image.png" }
 LayeredTexture: 5, "LayeredTexture::Layer", "" { BlendModes: *1 { a: 30 }
  Alphas: *1 { a: 0.5 } }
 NodeAttribute: 6, "NodeAttribute::Camera", "Camera" { }
 Model: 7, "Model::Camera", "Camera" { }
 NodeAttribute: 8, "NodeAttribute::Light", "Light" { }
 Model: 9, "Model::Light", "Light" { }
 Model: 10, "Model::Bone", "LimbNode" { }
 NodeAttribute: 11, "NodeAttribute::Bone", "LimbNode" { }
 Deformer: 12, "Deformer::Skin", "Skin" { SkinningType: "Linear" }
 Deformer: 13, "SubDeformer::Cluster", "Cluster" {
  Indexes: *1 { a: 0 }
  Weights: *1 { a: 1 }
  Transform: *16 { a: 1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1 }
  TransformLink: *16 { a: 1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1 }
 }
 Deformer: 14, "Deformer::Blend", "BlendShape" { }
 Deformer: 15, "SubDeformer::Channel", "BlendShapeChannel" {
  DeformPercent: 50
  FullWeights: *1 { a: 100 }
 }
 Geometry: 16, "Geometry::Shape", "Shape" {
  Indexes: *1 { a: 0 }
  Vertices: *3 { a: 1,0,0 }
 }
 Pose: 17, "Pose::Bind", "BindPose" {
  Type: "BindPose"
  NbPoseNodes: 1
  PoseNode: { Node: 10
   Matrix: *16 { a: 1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1 } }
 }
}
Connections: {
 C: "OO",1,2
 C: "OO",2,0
 C: "OO",3,2
 C: "OP",5,3,"UnmappedAttachment"
 C: "OO",4,5
 C: "OO",6,7
 C: "OO",7,0
 C: "OO",8,9
 C: "OO",9,0
 C: "OO",10,0
 C: "OO",11,10
 C: "OO",12,1
 C: "OO",13,12
 C: "OO",10,13
 C: "OO",14,1
 C: "OO",15,14
 C: "OO",16,15
}
"#;
    #[test]
    fn each_projection_reservation_refusal_is_typed() {
        let limits = Limits::default().dump_limits();
        let mut success = None;
        for n in 0..1000 {
            let c = canon_scene::project_with(FIXTURE, &limits, true).unwrap();
            crate::api_alloc::FAIL_AFTER.with(|f| f.set(Some(n)));
            let result = Scene::from_projection(c, &limits);
            crate::api_alloc::FAIL_AFTER.with(|f| f.set(None));
            match result {
                Ok(_) => {
                    success = Some(n);
                    break;
                }
                Err(e) => assert_eq!(e, Error::Allocation, "reservation {n}"),
            }
        }
        assert!(success.is_some_and(|n| n >= 30), "{success:?}");
        eprintln!(
            "projection reservation sweep: {} refused stages then success",
            success.unwrap()
        );
    }
    #[test]
    fn material_source_fallback_and_texture_links() {
        let scene = load(FIXTURE, &LoadOptions::default()).unwrap();
        assert_eq!(scene.bones().len(), 1);
        assert_eq!(scene.skin_deformers().len(), 1);
        assert_eq!(scene.blend_shapes().len(), 1);
        assert_eq!(scene.poses().len(), 1);
        assert_eq!(scene.meshes()[0].uv_sets().len(), 1);
        assert_eq!(scene.meshes()[0].color_sets().len(), 1);
        let m = &scene.materials()[0];
        assert_eq!(m.features().count(), 23);
        assert_eq!(m.source_property(b"Opacity").unwrap().value_real[0], 0.25);
        assert!(!m.feature("double_sided").unwrap().enabled);
        assert!(m.feature("diffuse").unwrap().enabled);
        assert!(m.feature("specular").unwrap().enabled);
        assert!(!m.feature("specular").unwrap().is_explicit);
        assert_eq!(m.textures().len(), 1);
        assert_eq!(m.textures()[0].material_prop, b"UnmappedAttachment");
        let layered = &scene.textures()[m.textures()[0].texture];
        assert!(layered.is_layered());
        assert_eq!(layered.layers().len(), 1);
        assert_eq!(layered.layers()[0].alpha, 0.5);
        // The pinned loader ignores array blend mode 30 (strictly < Overlay).
        assert_eq!(layered.layers()[0].blend_mode, 0);
        assert_eq!(layered.file_textures(), &[layered.layers()[0].texture]);
    }
    #[test]
    fn source_template_fallback_preserves_authored_priority() {
        let input = std::str::from_utf8(FIXTURE).unwrap().replace(
            "Objects: {",
            r#"Definitions: {
 ObjectType: "Material" {
  PropertyTemplate: "FbxSurfacePhong" {
   Properties70: {
    P: "Opacity", "Number", "", "", 0.75
    P: "TemplateOnly", "Number", "", "", 42
    P: "Label", "KString", "", "", "from template"
   }
  }
 }
}
Objects: {"#,
        );
        let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
        let m = &s.materials()[0];
        assert_eq!(m.source_property(b"Opacity").unwrap().value_real[0], 0.25);
        assert_eq!(
            m.source_property(b"TemplateOnly").unwrap().value_real[0],
            42.0
        );
        assert_eq!(
            m.source_property(b"Label").unwrap().value_str,
            b"from template"
        );
        assert!(m.source_property(b"missing").is_none());
    }
}

#[cfg(test)]
mod tests {
    /// A left-handed source (front axis -Z) is mirrored into the right-handed
    /// target: faces are flipped (first corner kept, the rest reversed), and the
    /// triangulation of the source quad is remapped onto the flipped corners so
    /// every triangle still names the same source vertices.
    #[test]
    fn reversed_winding_remaps_triangle_corners() {
        let fbx = br#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
    FBXVersion: 7400
}
GlobalSettings:  {
    Version: 1000
    Properties70:  {
        P: "UpAxis", "int", "Integer", "",1
        P: "UpAxisSign", "int", "Integer", "",1
        P: "FrontAxis", "int", "Integer", "",2
        P: "FrontAxisSign", "int", "Integer", "",-1
        P: "CoordAxis", "int", "Integer", "",0
        P: "CoordAxisSign", "int", "Integer", "",1
    }
}
Objects:  {
    Geometry: 1, "Geometry::Quad", "Mesh" {
        Vertices: *12 { a: 0,0,0, 2,0,0, 2,1,0, 0,1,0 }
        PolygonVertexIndex: *4 { a: 0,1,2,-4 }
    }
    Model: 2, "Model::Quad", "Mesh" {
    }
}
Connections:  {
    C: "OO",1,2
    C: "OO",2,0
}
"#;
        let scene = crate::load(fbx, &crate::LoadOptions::default()).expect("load");
        let mesh = &scene.meshes()[0];
        assert!(mesh.reversed_winding());
        let corner_vertex = mesh.positions().indices();
        // Flipped face: corners 0..4 address vertices 0, 3, 2, 1.
        assert_eq!(corner_vertex, &[0, 3, 2, 1]);
        // ufbx splits the source quad into (0 1 2) (2 3 0); on the flipped
        // corners those are corners (0 3 2) (2 1 0).
        assert_eq!(mesh.triangles(), &[0, 3, 2, 2, 1, 0]);
        let vertices: Vec<u32> = mesh
            .triangles()
            .iter()
            .map(|&c| corner_vertex[c as usize])
            .collect();
        assert_eq!(vertices, [0, 1, 2, 2, 3, 0]);
    }
}
