//! Experimental borrowed mesh and layer extraction. This is not a complete mesh converter.
use crate::scene::{ConnectionKind, Error, SceneIndex};
use crate::syntax::{AsciiValue, Document, Property};
use std::mem::size_of;

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_meshes: usize,
    pub max_layers: usize,
    pub max_allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_meshes: 1 << 22,
            max_layers: 1 << 23,
            max_allocation: crate::allocation::DEFAULT_MAX_ALLOCATION,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    ByPolygonVertex,
    ByVertex,
    ByPolygon,
    ByEdge,
    AllSame,
    /// `NoMappingInformation` or any mode this reader does not recognise.
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    Direct,
    IndexToDirect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Normal,
    Tangent,
    Binormal,
    Uv,
    Color,
    Material,
    Smoothing,
}
#[derive(Debug, Clone, Copy)]
pub enum NumericArray<'a> {
    Property(&'a Property<'a>),
    Scalars(&'a [Property<'a>]),
}
impl NumericArray<'_> {
    pub fn len(&self) -> usize {
        match self {
            Self::Scalars(v) => v.len(),
            Self::Property(p) => match p {
                Property::Array { count, .. } => *count,
                Property::AsciiArray(v) => v.len(),
                _ => 0,
            },
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn word(&self, index: usize) -> Option<(u8, u64)> {
        let Self::Property(p) = self else {
            return None;
        };
        let Property::Array {
            kind,
            count,
            big_endian,
            bytes,
        } = p
        else {
            return None;
        };
        if index >= *count {
            return None;
        }
        let width = match kind {
            b'd' | b'l' => 8,
            b'f' | b'i' => 4,
            b'b' | b'c' => 1,
            _ => return None,
        };
        let offset = index.checked_mul(width)?;
        let part = bytes.get(offset..offset.checked_add(width)?)?;
        // Dispatch once per word and let the byte conversion compile to a
        // native load/byte-swap, rather than accumulating one byte at a time.
        let value = match width {
            8 => {
                let part = part.try_into().ok()?;
                if *big_endian {
                    u64::from_be_bytes(part)
                } else {
                    u64::from_le_bytes(part)
                }
            }
            4 => {
                let part = part.try_into().ok()?;
                u64::from(if *big_endian {
                    u32::from_be_bytes(part)
                } else {
                    u32::from_le_bytes(part)
                })
            }
            1 => u64::from(part[0]),
            _ => return None,
        };
        Some((*kind, value))
    }
    pub fn f64(&self, index: usize) -> Option<f64> {
        let p = match self {
            Self::Scalars(v) => {
                return match v.get(index)? {
                    Property::I16(v) => Some(f64::from(*v)),
                    Property::I32(v) => Some(f64::from(*v)),
                    Property::I64(v) => Some(*v as f64),
                    Property::F32(v) => Some(f64::from(f32::from_bits(*v))),
                    Property::F64(v) => Some(f64::from_bits(*v)),
                    Property::AsciiNegativeZero => Some(-0.0),
                    _ => None,
                }
            }
            Self::Property(p) => p,
        };
        match p {
            Property::AsciiArray(a) => match a.get(index)? {
                AsciiValue::I64(v) => Some(*v as f64),
                AsciiValue::F64(v) => Some(f64::from_bits(*v)),
                AsciiValue::NegativeZero => Some(-0.0),
                AsciiValue::Bare(w) => Some(crate::syntax::bare_number(w).1),
            },
            _ => match self.word(index)? {
                (b'd', v) => Some(f64::from_bits(v)),
                (b'f', v) => Some(f64::from(f32::from_bits(u32::try_from(v).ok()?))),
                (b'i', v) => {
                    Some((i32::from_le_bytes(v.to_le_bytes()[..4].try_into().unwrap())) as f64)
                }
                (b'l', v) => Some((i64::from_ne_bytes(v.to_ne_bytes())) as f64),
                (b'b' | b'c', v) => Some(v as f64),
                _ => None,
            },
        }
    }
    pub fn i64(&self, index: usize) -> Option<i64> {
        let p = match self {
            Self::Scalars(v) => {
                return match v.get(index)? {
                    Property::I16(v) => Some(i64::from(*v)),
                    Property::I32(v) => Some(i64::from(*v)),
                    Property::I64(v) => Some(*v),
                    _ => None,
                }
            }
            Self::Property(p) => p,
        };
        match p {
            Property::AsciiArray(a) => match a.get(index)? {
                AsciiValue::I64(v) => Some(*v),
                AsciiValue::NegativeZero => Some(0),
                AsciiValue::Bare(w) => Some(crate::syntax::bare_number(w).0),
                _ => None,
            },
            _ => match self.word(index)? {
                (b'i', v) => Some(i64::from(i32::from_le_bytes(
                    v.to_le_bytes()[..4].try_into().unwrap(),
                ))),
                (b'l', v) => Some(i64::from_ne_bytes(v.to_ne_bytes())),
                (b'b' | b'c', v) => Some(i64::from_ne_bytes(v.to_ne_bytes())),
                _ => None,
            },
        }
    }
}
#[derive(Debug)]
pub struct Mesh<'a> {
    pub object: usize,
    pub node: usize,
    pub vertices: Option<NumericArray<'a>>,
    pub polygon_indices: Option<NumericArray<'a>>,
    pub edges: Option<NumericArray<'a>>,
}
#[derive(Debug)]
pub struct Layer<'a> {
    pub mesh: usize,
    pub node: usize,
    pub kind: LayerKind,
    pub typed_index: i64,
    pub name: Option<&'a [u8]>,
    pub mapping: Option<Mapping>,
    pub reference: Option<Reference>,
    pub direct: Option<NumericArray<'a>>,
    pub indices: Option<NumericArray<'a>>,
}
#[derive(Debug)]
pub struct Meshes<'a> {
    pub meshes: Vec<Mesh<'a>>,
    pub layers: Vec<Layer<'a>>,
}
#[derive(Debug, Clone, Copy)]
pub struct CornerContext {
    pub polygon_vertex: usize,
    pub vertex: usize,
    pub polygon: usize,
    pub edge: usize,
}
impl Layer<'_> {
    /// Resolve the direct-array entry for a corner. Component width is layer-specific.
    pub fn direct_index(&self, context: CornerContext) -> Result<usize, Error> {
        let mapped = match self.mapping.ok_or(Error::InvalidMesh { node: self.node })? {
            Mapping::ByPolygonVertex => context.polygon_vertex,
            Mapping::ByVertex => context.vertex,
            Mapping::ByPolygon => context.polygon,
            Mapping::ByEdge => context.edge,
            Mapping::AllSame => 0,
            Mapping::None => return Err(Error::InvalidMesh { node: self.node }),
        };
        // ufbx never reads the label (the string "ReferenceInformationType"
        // does not occur in ufbx.c): a layer is indexed exactly when it has an
        // index array (`ufbxi_read_vertex_element`, ufbx.c line 12743). A missing
        // or unrecognised label resolves the same way.
        let reference = self.reference.unwrap_or(if self.indices.is_some() {
            Reference::IndexToDirect
        } else {
            Reference::Direct
        });
        match reference {
            Reference::Direct => Ok(mapped),
            Reference::IndexToDirect => {
                let v = self
                    .indices
                    .ok_or(Error::InvalidMesh { node: self.node })?
                    .i64(mapped)
                    .ok_or(Error::InvalidMesh { node: self.node })?;
                usize::try_from(v).map_err(|_| Error::InvalidMesh { node: self.node })
            }
        }
    }
}
fn bytes<'a>(p: &'a Property<'_>) -> Option<&'a [u8]> {
    match p {
        Property::String(v) => Some(v),
        Property::AsciiBare(v) => Some(v),
        _ => None,
    }
}
fn integer(p: &Property<'_>) -> Option<i64> {
    match p {
        Property::I16(v) => Some(i64::from(*v)),
        Property::I32(v) => Some(i64::from(*v)),
        Property::I64(v) => Some(*v),
        _ => None,
    }
}
fn array<'a>(doc: &'a Document<'a>, node: usize) -> Option<NumericArray<'a>> {
    let n = doc.nodes.get(node)?;
    if let Some(p @ (Property::Array { .. } | Property::AsciiArray(_))) =
        doc.properties.get(n.properties.clone())?.first()
    {
        return Some(NumericArray::Property(p));
    }
    let values = doc.properties.get(n.properties.clone())?;
    if doc.version < 7000 && !values.is_empty() {
        return Some(NumericArray::Scalars(values));
    }
    let child = doc.nodes.get(node.checked_add(1)?)?;
    if child.parent != Some(node) || child.name != b"a" {
        return None;
    }
    match doc.properties.get(child.properties.clone())?.first()? {
        p @ (Property::Array { .. } | Property::AsciiArray(_)) => Some(NumericArray::Property(p)),
        _ => None,
    }
}
fn first_bytes<'a>(doc: &'a Document<'a>, node: usize) -> Option<&'a [u8]> {
    bytes(
        doc.properties
            .get(doc.nodes.get(node)?.properties.clone())?
            .first()?,
    )
}
fn layer_kind(name: &[u8]) -> Option<LayerKind> {
    match name {
        b"LayerElementNormal" => Some(LayerKind::Normal),
        b"LayerElementTangent" => Some(LayerKind::Tangent),
        b"LayerElementBinormal" => Some(LayerKind::Binormal),
        b"LayerElementUV" => Some(LayerKind::Uv),
        b"LayerElementColor" => Some(LayerKind::Color),
        b"LayerElementMaterial" => Some(LayerKind::Material),
        b"LayerElementSmoothing" => Some(LayerKind::Smoothing),
        _ => None,
    }
}
/// Classify a `MappingInformationType` string. ufbx 0.23.1 compares it against
/// exactly these names (`ufbxi_read_vertex_element`, ufbx.c lines 12797 and
/// 12872 for the `ByVertex`/`ByVertice` spelling pair) and ignores a layer
/// element whose mapping is anything else, with a missing-polygon-mapping
/// warning (ufbx.c lines 12850 and 12903): there is no `ByControlPoint` alias
/// in ufbx. Any other string therefore maps to [`Mapping::None`] instead of
/// failing the mesh.
fn mapping(s: &[u8]) -> Mapping {
    match s {
        b"ByPolygonVertex" => Mapping::ByPolygonVertex,
        b"ByVertice" | b"ByVertex" => Mapping::ByVertex,
        b"ByPolygon" => Mapping::ByPolygon,
        b"ByEdge" => Mapping::ByEdge,
        b"AllSame" => Mapping::AllSame,
        _ => Mapping::None,
    }
}
/// The authored reference label. `None` for a label that is not `Direct`,
/// `Index` or `IndexToDirect`: ufbx (which does not read the label at all, see
/// [`Layer::direct_index`]) keeps the mesh, so that label must not fail it.
fn reference(s: &[u8]) -> Option<Reference> {
    match s {
        b"Direct" => Some(Reference::Direct),
        b"Index" | b"IndexToDirect" => Some(Reference::IndexToDirect),
        _ => None,
    }
}
/// Extract arrays and layer declarations; validation and triangulation are separate.
pub fn extract<'a>(
    doc: &'a Document<'a>,
    index: &SceneIndex<'a>,
    limits: &Limits,
) -> Result<Meshes<'a>, Error> {
    let mut mesh_count = 0usize;
    for obj in &index.objects {
        let n = doc.nodes.get(obj.node).ok_or(Error::InvalidDom)?;
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        if (n.name == b"Geometry" || (doc.version < 7000 && n.name == b"Model"))
            && (p
                .get(if doc.version < 7000 { 1 } else { 2 })
                .and_then(bytes)
                .map(crate::scene::fbx_subtype)
                == Some(b"Mesh")
                || (doc.version < 7000
                    && doc
                        .nodes
                        .iter()
                        .skip(obj.node + 1)
                        .take_while(|c| c.parent.is_some_and(|p| p >= obj.node))
                        .any(|c| c.parent == Some(obj.node) && c.name == b"Vertices")))
        {
            mesh_count += 1;
        }
    }
    let layer_count = doc
        .nodes
        .iter()
        .filter(|n| layer_kind(n.name).is_some())
        .count();
    if mesh_count > limits.max_meshes {
        return Err(Error::LimitExceeded("meshes"));
    }
    if layer_count > limits.max_layers {
        return Err(Error::LimitExceeded("layers"));
    }
    let requested = mesh_count
        .checked_mul(size_of::<Mesh<'_>>())
        .and_then(|v| {
            layer_count
                .checked_mul(size_of::<Layer<'_>>())
                .and_then(|w| v.checked_add(w))
        })
        .ok_or(Error::LimitExceeded("allocation"))?;
    if requested > limits.max_allocation {
        return Err(Error::LimitExceeded("allocation"));
    }
    let mut meshes = Vec::new();
    let mut layers = Vec::new();
    let mut used = 0;
    crate::scene::reserve(&mut meshes, mesh_count, &mut used, limits.max_allocation)?;
    crate::scene::reserve(&mut layers, layer_count, &mut used, limits.max_allocation)?;
    for (object, obj) in index.objects.iter().enumerate() {
        let n = &doc.nodes[obj.node];
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        if (n.name == b"Geometry" || (doc.version < 7000 && n.name == b"Model"))
            && (p
                .get(if doc.version < 7000 { 1 } else { 2 })
                .and_then(bytes)
                .map(crate::scene::fbx_subtype)
                == Some(b"Mesh")
                || (doc.version < 7000
                    && doc
                        .nodes
                        .iter()
                        .skip(obj.node + 1)
                        .take_while(|c| c.parent.is_some_and(|p| p >= obj.node))
                        .any(|c| c.parent == Some(obj.node) && c.name == b"Vertices")))
        {
            meshes.push(Mesh {
                object,
                node: obj.node,
                vertices: None,
                polygon_indices: None,
                edges: None,
            });
        }
    }
    for (i, n) in doc.nodes.iter().enumerate() {
        let Some(parent) = n.parent else { continue };
        if let Ok(mesh) = meshes.binary_search_by_key(&parent, |m| m.node) {
            if let Some(kind) = layer_kind(n.name) {
                let p = doc
                    .properties
                    .get(n.properties.clone())
                    .ok_or(Error::InvalidDom)?;
                layers.push(Layer {
                    mesh,
                    node: i,
                    kind,
                    typed_index: p.first().and_then(integer).unwrap_or(0),
                    name: None,
                    mapping: None,
                    reference: None,
                    direct: None,
                    indices: None,
                });
            } else {
                let dest = match n.name {
                    b"Vertices" => Some(&mut meshes[mesh].vertices),
                    b"PolygonVertexIndex" => Some(&mut meshes[mesh].polygon_indices),
                    b"Edges" => Some(&mut meshes[mesh].edges),
                    _ => None,
                };
                if let Some(dest) = dest {
                    *dest = Some(array(doc, i).ok_or(Error::InvalidMesh { node: i })?);
                }
            }
        } else if let Ok(layer) = layers.binary_search_by_key(&parent, |l| l.node) {
            let l = &mut layers[layer];
            match n.name {
                b"Name" => {
                    l.name = Some(first_bytes(doc, i).ok_or(Error::InvalidMesh { node: i })?)
                }
                b"MappingInformationType" => {
                    l.mapping = Some(mapping(
                        first_bytes(doc, i).ok_or(Error::InvalidMesh { node: i })?,
                    ))
                }
                b"ReferenceInformationType" => {
                    // An unrecognised label is not an error: see `reference`.
                    l.reference =
                        reference(first_bytes(doc, i).ok_or(Error::InvalidMesh { node: i })?)
                }
                b"Normals" | b"Tangents" | b"Binormals" | b"UV" | b"Colors" | b"Materials"
                | b"Smoothing" => {
                    l.direct = Some(array(doc, i).ok_or(Error::InvalidMesh { node: i })?)
                }
                b"NormalsIndex" | b"NormalIndex" | b"TangentsIndex" | b"TangentIndex"
                | b"BinormalsIndex" | b"BinormalIndex" | b"UVIndex" | b"ColorIndex"
                | b"ColorsIndex" | b"MaterialIndex" | b"SmoothingIndex" => {
                    l.indices = Some(array(doc, i).ok_or(Error::InvalidMesh { node: i })?)
                }
                _ => (),
            }
        }
    }
    Ok(Meshes { meshes, layers })
}
impl Meshes<'_> {
    /// Iterate object indices of model instances attached to a mesh.
    pub fn instances<'a>(
        &'a self,
        mesh: usize,
        index: &'a SceneIndex<'_>,
        doc: &'a Document<'_>,
    ) -> impl Iterator<Item = usize> + 'a {
        let object = self.meshes.get(mesh).map(|m| m.object);
        index.connections.iter().filter_map(move |c| {
            if Some(c.source) == object && c.kind == ConnectionKind::Object {
                c.destination.filter(|&d| {
                    index
                        .objects
                        .get(d)
                        .and_then(|o| doc.nodes.get(o.node))
                        .is_some_and(|n| n.name == b"Model")
                })
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Topology {
    pub vertices: usize,
    pub corners: usize,
    pub faces: usize,
}
impl Mesh<'_> {
    /// Validate authored control-point and polygon-corner indices. This does not
    /// repair malformed polygons or prove any particular triangulation policy.
    pub fn validate_topology(&self) -> Result<Topology, Error> {
        let invalid = || Error::InvalidMesh { node: self.node };
        let vertices = self.vertices.ok_or_else(invalid)?;
        let corners = self.polygon_indices.ok_or_else(invalid)?;
        if vertices.len() % 3 != 0 {
            return Err(invalid());
        }
        let num_vertices = vertices.len() / 3;
        for i in 0..vertices.len() {
            vertices.f64(i).ok_or_else(invalid)?;
        }
        let mut faces = 0usize;
        let mut open = false;
        for i in 0..corners.len() {
            let v = corners.i64(i).ok_or_else(invalid)?;
            let end = v < 0;
            let vertex = if end { !v } else { v };
            if usize::try_from(vertex)
                .ok()
                .filter(|v| *v < num_vertices)
                .is_none()
            {
                return Err(invalid());
            }
            open = !end;
            if end {
                faces = faces.checked_add(1).ok_or_else(invalid)?;
            }
        }
        if open {
            return Err(invalid());
        }
        Ok(Topology {
            vertices: num_vertices,
            corners: corners.len(),
            faces,
        })
    }
}
impl Layer<'_> {
    pub fn components(&self) -> usize {
        match self.kind {
            LayerKind::Normal | LayerKind::Tangent | LayerKind::Binormal => 3,
            LayerKind::Uv => 2,
            LayerKind::Color => 4,
            LayerKind::Material | LayerKind::Smoothing => 1,
        }
    }
    /// Get the first scalar of a mapped direct entry after validating its width.
    pub fn direct_offset(&self, context: CornerContext) -> Result<usize, Error> {
        let invalid = || Error::InvalidMesh { node: self.node };
        let width = self.components();
        let start = self
            .direct_index(context)?
            .checked_mul(width)
            .ok_or_else(invalid)?;
        let end = start.checked_add(width).ok_or_else(invalid)?;
        if end > self.direct.ok_or_else(invalid)?.len() {
            return Err(invalid());
        }
        Ok(start)
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalized(a: [f64; 3]) -> [f64; 3] {
    let len = dot(a, a).sqrt();
    if len > 1.4916681462400413e-154 {
        let inv = 1.0 / len;
        [a[0] * inv, a[1] * inv, a[2] * inv]
    } else {
        [0.0; 3]
    }
}
impl Mesh<'_> {
    fn corner_position(&self, corner: usize) -> Option<[f64; 3]> {
        let v = self.polygon_indices?.i64(corner)?;
        let index = usize::try_from(if v < 0 { !v } else { v })
            .ok()?
            .checked_mul(3)?;
        let positions = self.vertices?;
        Some([
            positions.f64(index)?,
            positions.f64(index.checked_add(1)?)?,
            positions.f64(index.checked_add(2)?)?,
        ])
    }
    /// Triangulate a triangle or quad into source polygon-corner indices.
    /// N-gons are rejected until the reference ear-clipping algorithm is ported.
    /// Algorithm and ordering: ufbx 0.23.1 `ufbx_catch_triangulate_face` (MIT),
    /// <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
    pub fn triangulate_face_into(
        &self,
        begin: usize,
        count: usize,
        output: &mut [usize],
    ) -> Result<usize, Error> {
        let invalid = || Error::InvalidMesh { node: self.node };
        let corners = self.polygon_indices.ok_or_else(invalid)?;
        let end = begin.checked_add(count).ok_or_else(invalid)?;
        if end > corners.len() {
            return Err(invalid());
        }
        if count < 3 {
            return Ok(0);
        }
        if count > 4 {
            return Err(Error::UnsupportedTriangulation);
        }
        let required = (count - 2) * 3;
        if output.len() < required {
            return Err(Error::BufferTooSmall { required });
        }
        if count == 3 {
            for corner in begin..end {
                self.corner_position(corner).ok_or_else(invalid)?;
            }
            output[..3].copy_from_slice(&[begin, begin + 1, begin + 2]);
            return Ok(1);
        }
        let [v0, v1, v2, v3] = [
            self.corner_position(begin).ok_or_else(invalid)?,
            self.corner_position(begin + 1).ok_or_else(invalid)?,
            self.corner_position(begin + 2).ok_or_else(invalid)?,
            self.corner_position(begin + 3).ok_or_else(invalid)?,
        ];
        let a = sub(v2, v0);
        let b = sub(v3, v1);
        let na1 = normalized(cross(a, sub(v1, v0)));
        let na3 = normalized(cross(a, sub(v0, v3)));
        let nb0 = normalized(cross(b, sub(v1, v0)));
        let nb2 = normalized(cross(b, sub(v2, v1)));
        let (aa, bb, na, nb) = (dot(a, a), dot(b, b), dot(na1, na3), dot(nb0, nb2));
        let split_a = if na < 0.0 || nb < 0.0 {
            na >= nb
        } else {
            aa <= bb
        };
        let indices = if split_a {
            [0, 1, 2, 2, 3, 0]
        } else {
            [1, 2, 3, 3, 0, 1]
        };
        for (dst, index) in output.iter_mut().zip(indices) {
            *dst = begin + index;
        }
        Ok(2)
    }
}

/// Per-face bounds for the n-gon triangulation workspace and candidate tests.
#[derive(Debug, Clone)]
pub struct TriangulationLimits {
    pub max_face_vertices: usize,
    pub max_allocation: usize,
    pub max_work: usize,
}
impl Default for TriangulationLimits {
    fn default() -> Self {
        Self {
            max_face_vertices: 1 << 20,
            max_allocation: 128 << 20,
            max_work: 1 << 28,
        }
    }
}
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    let x = a[0] - b[0];
    let y = a[1] - b[1];
    x * x + y * y
}
fn ear_weight(p: [[f64; 2]; 3]) -> f64 {
    if orient(p[0], p[1], p[2]) <= 0.0 {
        return -1.0;
    }
    let a = distance2(p[0], p[1]);
    let b = distance2(p[1], p[2]);
    let c = distance2(p[2], p[0]);
    let ab = (a + b - c) / (4.0 * a * b).sqrt();
    let bc = (b + c - a) / (4.0 * b * c).sqrt();
    let ca = (c + a - b) / (4.0 * c * a).sqrt();
    1.4916681462400413e-154f64.max(2.0 - ab.max(bc).max(ca))
}
impl Mesh<'_> {
    /// Bounded triangulation with ufbx 0.23.1's ear choice and output ordering.
    /// Uses a bounded linear scan of reflex points in place of ufbx's KD tree.
    /// The algorithm is derived from MIT-licensed `ufbxi_triangulate_ngon`:
    /// <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
    pub fn triangulate_with_limits(
        &self,
        begin: usize,
        count: usize,
        output: &mut [usize],
        limits: &TriangulationLimits,
    ) -> Result<usize, Error> {
        if count <= 4 {
            return self.triangulate_face_into(begin, count, output);
        }
        let invalid = || Error::InvalidMesh { node: self.node };
        if count > limits.max_face_vertices || count >= (1usize << 31) {
            return Err(Error::LimitExceeded("face vertices"));
        }
        let end = begin.checked_add(count).ok_or_else(invalid)?;
        if end > self.polygon_indices.ok_or_else(invalid)?.len() {
            return Err(invalid());
        }
        let required = count
            .checked_sub(2)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(invalid)?;
        if output.len() < required {
            return Err(Error::BufferTooSmall { required });
        }
        let per = size_of::<[f64; 2]>()
            + size_of::<[usize; 2]>()
            + size_of::<usize>()
            + size_of::<bool>();
        if count
            .checked_mul(per)
            .ok_or(Error::LimitExceeded("allocation"))?
            > limits.max_allocation
        {
            return Err(Error::LimitExceeded("allocation"));
        }
        let mut points = Vec::new();
        let mut edges = Vec::new();
        let mut reflex = Vec::new();
        let mut clipped = Vec::new();
        let mut used = 0;
        crate::scene::reserve(&mut points, count, &mut used, limits.max_allocation)?;
        crate::scene::reserve(&mut edges, count, &mut used, limits.max_allocation)?;
        crate::scene::reserve(&mut reflex, count, &mut used, limits.max_allocation)?;
        crate::scene::reserve(&mut clipped, count, &mut used, limits.max_allocation)?;
        let mut work = 0usize;
        let mut charge = |amount: usize| -> Result<(), Error> {
            work = work
                .checked_add(amount)
                .ok_or(Error::LimitExceeded("triangulation work"))?;
            if work > limits.max_work {
                Err(Error::LimitExceeded("triangulation work"))
            } else {
                Ok(())
            }
        };
        let mut normal = [0.0; 3];
        charge(count)?;
        for i in 0..count {
            let a = self.corner_position(begin + i).ok_or_else(invalid)?;
            let b = self
                .corner_position(begin + if i + 1 < count { i + 1 } else { 0 })
                .ok_or_else(invalid)?;
            if a.iter().chain(b.iter()).any(|v| !v.is_finite()) {
                return Err(invalid());
            }
            normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
            normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
            normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let len = dot(normal, normal).sqrt();
        normal = if len > 1.4916681462400413e-154 {
            let k = 1.0 / len;
            [normal[0] * k, normal[1] * k, normal[2] * k]
        } else {
            [1.0, 0.0, 0.0]
        };
        let axis = if normal[0] * normal[0] < 0.5 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let x = normalized(cross(axis, normal));
        let y = normalized(cross(normal, x));
        charge(count)?;
        for i in 0..count {
            let p = self.corner_position(begin + i).ok_or_else(invalid)?;
            points.push([dot(x, p), dot(y, p)]);
            edges.push([
                if i > 0 { i - 1 } else { count - 1 },
                if i + 1 < count { i + 1 } else { 0 },
            ]);
            clipped.push(false);
        }
        charge(count)?;
        for i in 0..count {
            if orient(
                points[if i > 0 { i - 1 } else { count - 1 }],
                points[i],
                points[if i + 1 < count { i + 1 } else { 0 }],
            ) <= 0.0
            {
                reflex.push(i);
            }
        }
        let mut remaining = count;
        let mut p = [0usize, 1, 2, 3];
        let mut steps = 0usize;
        while remaining > 3 {
            charge(1)?;
            let weights = [
                ear_weight([points[p[0]], points[p[1]], points[p[2]]]),
                ear_weight([points[p[1]], points[p[2]], points[p[3]]]),
            ];
            let first = usize::from(weights[1] > weights[0]);
            let mut did_clip = false;
            for side_index in 0..2 {
                let side = side_index ^ first;
                if !matches!(
                    weights[side].partial_cmp(&0.0),
                    Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater)
                ) {
                    break;
                }
                let ids = [p[side], p[side + 1], p[side + 2]];
                let tri = [points[ids[0]], points[ids[1]], points[ids[2]]];
                let min = [
                    tri[0][0].min(tri[1][0]).min(tri[2][0]),
                    tri[0][1].min(tri[1][1]).min(tri[2][1]),
                ];
                let max = [
                    tri[0][0].max(tri[1][0]).max(tri[2][0]),
                    tri[0][1].max(tri[1][1]).max(tri[2][1]),
                ];
                let mut contains = false;
                for &r in &reflex {
                    charge(1)?;
                    if ids.contains(&r) {
                        continue;
                    }
                    let q = points[r];
                    if q[0] < min[0] || q[0] > max[0] || q[1] < min[1] || q[1] > max[1] {
                        continue;
                    }
                    let (u, v, w) = (
                        orient(q, tri[0], tri[1]),
                        orient(q, tri[1], tri[2]),
                        orient(q, tri[2], tri[0]),
                    );
                    if (u <= 0.0 && v <= 0.0 && w <= 0.0) || (u >= 0.0 && v >= 0.0 && w >= 0.0) {
                        contains = true;
                        break;
                    }
                }
                if !contains {
                    let [a, b, c] = ids;
                    clipped[b] = true;
                    edges[c][0] = a;
                    edges[a][1] = c;
                    remaining -= 1;
                    steps = 0;
                    if side == 1 {
                        p[2] = p[3];
                        p[3] = edges[p[3]][1];
                    } else {
                        p[1] = p[0];
                        p[0] = edges[p[0]][0];
                    }
                    did_clip = true;
                    break;
                }
            }
            if did_clip {
                continue;
            }
            p = [p[1], p[2], p[3], edges[p[3]][1]];
            steps += 1;
            if steps >= count.saturating_mul(2) {
                break;
            }
        }
        let mut ix = p[1];
        while remaining > 3 {
            charge(1)?;
            let [prev, next] = edges[ix];
            clipped[ix] = true;
            edges[prev][1] = next;
            edges[next][0] = prev;
            remaining -= 1;
            ix = next;
        }
        clipped[ix] = true;
        let mut written = 0;
        for i in 0..count {
            if clipped[i] {
                let [prev, next] = edges[i];
                output[written..written + 3].copy_from_slice(&[
                    begin + prev,
                    begin + i,
                    begin + next,
                ]);
                written += 3;
            }
        }
        if written != required {
            return Err(invalid());
        }
        Ok(count - 2)
    }
}

impl Mesh<'_> {
    /// Resolve an authored edge to the two polygon-corner indices it joins.
    /// Work is bounded when the edge closes a long polygon.
    pub fn edge_corners(&self, edge: usize, max_work: usize) -> Result<[usize; 2], Error> {
        let invalid = || Error::InvalidMesh { node: self.node };
        let raw = self
            .edges
            .ok_or_else(invalid)?
            .i64(edge)
            .ok_or_else(invalid)?;
        let at = usize::try_from(raw).map_err(|_| invalid())?;
        let corners = self.polygon_indices.ok_or_else(invalid)?;
        let value = corners.i64(at).ok_or_else(invalid)?;
        if value >= 0 {
            let next = at.checked_add(1).ok_or_else(invalid)?;
            if next >= corners.len() {
                return Err(invalid());
            }
            Ok([at, next])
        } else {
            let mut first = at;
            let mut work = 0;
            while first > 0 {
                work += 1;
                if work > max_work {
                    return Err(Error::LimitExceeded("edge work"));
                }
                if corners.i64(first - 1).ok_or_else(invalid)? < 0 {
                    break;
                }
                first -= 1;
            }
            Ok([at, first])
        }
    }
}
impl Layer<'_> {
    /// FBX material entries directly name a node's material slots even when the
    /// authored reference label is IndexToDirect (ufbx 0.23.1 mesh reader).
    pub fn material_index(&self, context: CornerContext) -> Result<usize, Error> {
        let invalid = || Error::InvalidMesh { node: self.node };
        if self.kind != LayerKind::Material {
            return Err(invalid());
        }
        let slot = match self.mapping.ok_or_else(invalid)? {
            Mapping::ByPolygonVertex => context.polygon_vertex,
            Mapping::ByVertex => context.vertex,
            Mapping::ByPolygon => context.polygon,
            Mapping::ByEdge => context.edge,
            Mapping::AllSame => 0,
            Mapping::None => return Err(invalid()),
        };
        let value = self
            .direct
            .ok_or_else(invalid)?
            .i64(slot)
            .ok_or_else(invalid)?;
        usize::try_from(value).map_err(|_| invalid())
    }
}

impl Meshes<'_> {
    /// Validate that each mapped layer entry addresses an available direct value.
    /// No ufbx-compatible repair or generated attribute is applied here.
    pub fn validate_layer(&self, layer_index: usize, max_work: usize) -> Result<usize, Error> {
        let layer = self.layers.get(layer_index).ok_or(Error::InvalidDom)?;
        let mesh = self.meshes.get(layer.mesh).ok_or(Error::InvalidDom)?;
        let invalid = || Error::InvalidMesh { node: layer.node };
        let base = mesh
            .vertices
            .ok_or_else(invalid)?
            .len()
            .checked_add(mesh.polygon_indices.ok_or_else(invalid)?.len())
            .ok_or(Error::LimitExceeded("layer work"))?;
        if base > max_work {
            return Err(Error::LimitExceeded("layer work"));
        }
        let topology = mesh.validate_topology()?;
        let count = match layer.mapping.ok_or_else(invalid)? {
            Mapping::ByPolygonVertex => topology.corners,
            Mapping::ByVertex => topology.vertices,
            Mapping::ByPolygon => topology.faces,
            Mapping::ByEdge => mesh.edges.ok_or_else(invalid)?.len(),
            Mapping::AllSame => 1,
            Mapping::None => 0,
        };
        let width = layer.components();
        let work = count
            .checked_mul(width)
            .ok_or(Error::LimitExceeded("layer work"))?;
        if base
            .checked_add(work)
            .ok_or(Error::LimitExceeded("layer work"))?
            > max_work
        {
            return Err(Error::LimitExceeded("layer work"));
        }
        for i in 0..count {
            let context = CornerContext {
                polygon_vertex: i,
                vertex: i,
                polygon: i,
                edge: i,
            };
            if layer.kind == LayerKind::Material {
                layer.material_index(context)?;
                continue;
            }
            let offset = layer.direct_offset(context)?;
            let direct = layer.direct.ok_or_else(invalid)?;
            for component in 0..width {
                direct.f64(offset + component).ok_or_else(invalid)?;
            }
        }
        Ok(count)
    }
}

/// Iterator over authored polygon ranges in polygon-corner order.
pub struct Faces<'a> {
    corners: NumericArray<'a>,
    next: usize,
    node: usize,
    finished: bool,
}
impl Mesh<'_> {
    pub fn faces(&self) -> Result<Faces<'_>, Error> {
        Ok(Faces {
            corners: self
                .polygon_indices
                .ok_or(Error::InvalidMesh { node: self.node })?,
            next: 0,
            node: self.node,
            finished: false,
        })
    }
}
impl Iterator for Faces<'_> {
    type Item = Result<std::ops::Range<usize>, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.next == self.corners.len() {
            return None;
        }
        let start = self.next;
        while self.next < self.corners.len() {
            let i = self.next;
            self.next += 1;
            match self.corners.i64(i) {
                Some(v) if v < 0 => return Some(Ok(start..self.next)),
                Some(_) => (),
                None => {
                    self.finished = true;
                    return Some(Err(Error::InvalidMesh { node: self.node }));
                }
            }
        }
        self.finished = true;
        Some(Err(Error::InvalidMesh { node: self.node }))
    }
}
