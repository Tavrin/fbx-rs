//! Preliminary, bounded FBX object and connection indexing.
//!
//! The static scene conversion itself lives in `canon_scene` and `api`.
use crate::syntax::{Document, Property};

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_objects: usize,
    pub max_connections: usize,
    /// Maximum bytes requested for conversion vectors, including temporary vectors.
    pub max_allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_objects: 1 << 20,
            max_connections: 1 << 22,
            max_allocation: 512 << 20,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDom,
    InvalidTransform,
    UnsupportedShader,
    AmbiguousTextureBinding,
    InvalidSurface { node: usize },
    InvalidEmbedded { node: usize },
    UnsupportedTriangulation,
    BufferTooSmall { required: usize },
    InvalidMesh { node: usize },
    InvalidObject { node: usize },
    InvalidConnection { node: usize },
    DuplicateId(i64),
    MissingId(i64),
    MultipleParents(i64),
    Cycle(i64),
    LimitExceeded(&'static str),
    AllocationFailed,
}
#[derive(Debug)]
pub struct Object {
    pub id: i64,
    /// Index into the original DOM. The DOM retains names and all authored data.
    pub node: usize,
    /// Parent object index for Model-to-Model OO connections; root is None.
    pub parent: Option<usize>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectionKind<'a> {
    Object,
    Property(&'a [u8]),
}
#[derive(Debug)]
pub struct Connection<'a> {
    pub kind: ConnectionKind<'a>,
    pub source: usize,
    /// None denotes the implicit FBX root ID 0.
    pub destination: Option<usize>,
}
/// Content categories recognized by the object index. The public loader
/// reports only categories it does not convert; skin, blend and pose tags
/// are retained here for compatibility with the lower-level index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnsupportedKind {
    /// `Deformer` of class `Skin` (converted by the public loader).
    Skin,
    /// `Deformer` of class `Cluster` (converted by the public loader).
    SkinCluster,
    /// `Deformer` of class `BlendShape` (converted by the public loader).
    BlendShape,
    /// `Deformer` of class `BlendShapeChannel` (converted by the public loader).
    BlendShapeChannel,
    /// `Geometry` of class `Shape`, a blend-shape target.
    ShapeGeometry,
    /// A `Shape` node nested in a `Geometry` object (FBX 7100-style in-geometry
    /// blend shape, converted into generated blend elements).
    LegacyShape,
    /// `AnimationStack` (animation, planned).
    AnimationStack,
    /// `AnimationLayer` (animation, planned).
    AnimationLayer,
    /// `AnimationCurveNode` (animation, planned).
    AnimationCurveNode,
    /// `AnimationCurve` (animation, planned).
    AnimationCurve,
    /// `Pose` (bind and rest poses, converted by the public loader).
    Pose,
    /// `Geometry` of class `NurbsCurve`, `NurbsSurface`, `TrimNurbsSurface`,
    /// `Boundary` or `Line` (not planned yet).
    CurveOrSurfaceGeometry,
    /// `Deformer` of class `VertexCacheDeformer` or a `Cache` object (not
    /// planned yet).
    Cache,
    /// `Constraint`, including characters (not planned yet).
    Constraint,
    /// `AudioLayer` or `Audio` (not planned yet).
    Audio,
}
#[derive(Debug)]
pub struct Unsupported {
    /// Index into [`SceneIndex::objects`].
    pub object: usize,
    pub kind: UnsupportedKind,
}
#[derive(Debug)]
pub struct SceneIndex<'a> {
    pub objects: Vec<Object>,
    pub connections: Vec<Connection<'a>>,
    pub unsupported: Vec<Unsupported>,
}
fn bytes<'a>(p: &'a Property<'_>) -> Option<&'a [u8]> {
    match p {
        Property::String(v) => Some(v.as_ref()),
        Property::AsciiBare(v) => Some(v),
        _ => None,
    }
}
pub(crate) fn fbx_subtype(value: &[u8]) -> &[u8] {
    if value.len() > 3 {
        value.strip_prefix(b"Fbx").unwrap_or(value)
    } else {
        value
    }
}
fn id(p: &Property<'_>) -> Option<i64> {
    match p {
        Property::I64(v) => Some(*v),
        Property::I32(v) => Some(i64::from(*v)),
        Property::I16(v) => Some(i64::from(*v)),
        _ => None,
    }
}
fn props<'a>(doc: &'a Document<'_>, index: usize) -> Result<&'a [Property<'a>], Error> {
    let node = doc.nodes.get(index).ok_or(Error::InvalidDom)?;
    doc.properties
        .get(node.properties.clone())
        .ok_or(Error::InvalidDom)
}
pub(crate) fn reserve<T>(
    v: &mut Vec<T>,
    n: usize,
    used: &mut usize,
    limit: usize,
) -> Result<(), Error> {
    crate::allocation::reserve(v, n, used, limit).map_err(|e| match e {
        crate::allocation::Error::Limit => Error::LimitExceeded("allocation"),
        crate::allocation::Error::Failed => Error::AllocationFailed,
    })
}
/// Index direct children of Objects and Connections; references borrow the input DOM.
/// Validates Model hierarchy cycles, not arbitrary non-hierarchy connection cycles.
pub fn index<'a>(doc: &'a Document<'a>, limits: &Limits) -> Result<SceneIndex<'a>, Error> {
    for (i, node) in doc.nodes.iter().enumerate() {
        if node.parent.is_some_and(|p| p >= i)
            || doc.properties.get(node.properties.clone()).is_none()
        {
            return Err(Error::InvalidDom);
        }
    }
    let is_child = |i: usize, name: &[u8]| {
        doc.nodes[i]
            .parent
            .and_then(|p| doc.nodes.get(p))
            .is_some_and(|p| p.parent.is_none() && p.name == name)
    };
    let object_count = (0..doc.nodes.len())
        .filter(|&i| is_child(i, b"Objects"))
        .count();
    let connection_count = (0..doc.nodes.len())
        .filter(|&i| is_child(i, b"Connections") && doc.nodes[i].name == b"C")
        .count();
    if object_count > limits.max_objects {
        return Err(Error::LimitExceeded("objects"));
    }
    if connection_count > limits.max_connections {
        return Err(Error::LimitExceeded("connections"));
    }
    let mut used = 0;
    let mut objects = Vec::new();
    let mut connections = Vec::new();
    let mut unsupported = Vec::new();
    let mut lookup: Vec<(i64, usize)> = Vec::new();
    let mut colors: Vec<u8> = Vec::new();
    reserve(&mut objects, object_count, &mut used, limits.max_allocation)?;
    reserve(&mut lookup, object_count, &mut used, limits.max_allocation)?;
    reserve(&mut colors, object_count, &mut used, limits.max_allocation)?;
    // FBX 7100-style blend shapes: `Shape` nodes directly inside a `Geometry`
    // object (ufbx `ufbxi_read_synthetic_blend_shapes`).
    let is_legacy_shape = |i: usize| {
        doc.nodes[i].name == b"Shape"
            && doc.nodes[i]
                .parent
                .is_some_and(|p| doc.nodes[p].name == b"Geometry" && is_child(p, b"Objects"))
    };
    let legacy_shapes = (0..doc.nodes.len()).filter(|&i| is_legacy_shape(i)).count();
    reserve(
        &mut unsupported,
        object_count
            .checked_add(legacy_shapes)
            .ok_or(Error::LimitExceeded("allocation"))?,
        &mut used,
        limits.max_allocation,
    )?;
    reserve(
        &mut connections,
        connection_count,
        &mut used,
        limits.max_allocation,
    )?;
    for i in 0..doc.nodes.len() {
        if !is_child(i, b"Objects") {
            continue;
        }
        let p = props(doc, i)?;
        let object_id = p
            .first()
            .and_then(id)
            .filter(|&v| v != 0)
            .ok_or(Error::InvalidObject { node: i })?;
        let index = objects.len();
        objects.push(Object {
            id: object_id,
            node: i,
            parent: None,
        });
        lookup.push((object_id, index));
        let kind = match (doc.nodes[i].name, p.get(2).and_then(bytes).map(fbx_subtype)) {
            (b"Deformer", Some(b"Skin")) => Some(UnsupportedKind::Skin),
            (b"Deformer", Some(b"Cluster")) => Some(UnsupportedKind::SkinCluster),
            (b"Deformer", Some(b"BlendShape")) => Some(UnsupportedKind::BlendShape),
            (b"Deformer", Some(b"BlendShapeChannel")) => Some(UnsupportedKind::BlendShapeChannel),
            (b"Geometry", Some(b"Shape")) => Some(UnsupportedKind::ShapeGeometry),
            (b"Geometry", Some(b"NurbsCurve" | b"NurbsSurface" | b"TrimNurbsSurface"))
            | (b"Geometry", Some(b"Boundary" | b"Line")) => {
                Some(UnsupportedKind::CurveOrSurfaceGeometry)
            }
            (b"Deformer", Some(b"VertexCacheDeformer")) | (b"Cache", _) => {
                Some(UnsupportedKind::Cache)
            }
            (b"AnimationStack", _) => Some(UnsupportedKind::AnimationStack),
            (b"AnimationLayer", _) => Some(UnsupportedKind::AnimationLayer),
            (b"AnimationCurveNode", _) => Some(UnsupportedKind::AnimationCurveNode),
            (b"AnimationCurve", _) => Some(UnsupportedKind::AnimationCurve),
            (b"Pose", _) => Some(UnsupportedKind::Pose),
            (b"Constraint", _) => Some(UnsupportedKind::Constraint),
            (b"AudioLayer" | b"Audio", _) => Some(UnsupportedKind::Audio),
            _ => None,
        };
        if let Some(kind) = kind {
            unsupported.push(Unsupported {
                object: index,
                kind,
            });
        }
    }
    if legacy_shapes > 0 {
        for i in 0..doc.nodes.len() {
            if !is_legacy_shape(i) {
                continue;
            }
            let geometry = doc.nodes[i].parent.unwrap_or(usize::MAX);
            // Objects were pushed in DOM order, so `node` is sorted.
            if let Ok(object) = objects.binary_search_by_key(&geometry, |o| o.node) {
                unsupported.push(Unsupported {
                    object,
                    kind: UnsupportedKind::LegacyShape,
                });
            }
        }
    }
    lookup.sort_unstable_by_key(|x| x.0);
    for pair in lookup.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(Error::DuplicateId(pair[0].0));
        }
    }
    let resolve = |v: i64| {
        lookup
            .binary_search_by_key(&v, |x| x.0)
            .map(|i| lookup[i].1)
            .map_err(|_| Error::MissingId(v))
    };
    colors.resize(object_count, 0);
    for i in 0..doc.nodes.len() {
        if !is_child(i, b"Connections") || doc.nodes[i].name != b"C" {
            continue;
        }
        let p = props(doc, i)?;
        let invalid = || Error::InvalidConnection { node: i };
        let tag = p.first().and_then(bytes).ok_or_else(invalid)?;
        let source_id = p.get(1).and_then(id).ok_or_else(invalid)?;
        let dest_id = p.get(2).and_then(id).ok_or_else(invalid)?;
        let kind = match tag {
            b"OO" => ConnectionKind::Object,
            b"OP" => ConnectionKind::Property(p.get(3).and_then(bytes).ok_or_else(invalid)?),
            _ => return Err(invalid()),
        };
        let source = resolve(source_id)?;
        let destination = if dest_id == 0 {
            None
        } else {
            Some(resolve(dest_id)?)
        };
        if kind == ConnectionKind::Object
            && doc.nodes[objects[source].node].name == b"Model"
            && (destination.is_none()
                || destination.is_some_and(|d| doc.nodes[objects[d].node].name == b"Model"))
        {
            if colors[source] != 0 {
                return Err(Error::MultipleParents(source_id));
            }
            colors[source] = 1;
            objects[source].parent = destination;
        }
        connections.push(Connection {
            kind,
            source,
            destination,
        });
    }
    colors.fill(0);
    for start in 0..object_count {
        let mut at = Some(start);
        while let Some(i) = at {
            if colors[i] == 2 {
                break;
            }
            if colors[i] == 1 {
                return Err(Error::Cycle(objects[i].id));
            }
            colors[i] = 1;
            at = objects[i].parent;
        }
        at = Some(start);
        while let Some(i) = at {
            if colors[i] != 1 {
                break;
            }
            colors[i] = 2;
            at = objects[i].parent;
        }
    }
    Ok(SceneIndex {
        objects,
        connections,
        unsupported,
    })
}
