//! Direct conversion of a loaded [`Mesh`] into meshoptimizer-rs inputs
//! (feature `meshoptimizer`).
//!
//! [`Mesh::to_meshopt`] validates the corners the triangles use, converts
//! them to 32-bit floats, welds corners whose emitted attributes are
//! bitwise equal, drops degenerate triangles and groups the triangle list by
//! material slot. Inconsistent or non-finite data is a [`MeshDataError`],
//! never a panic.
//!
//! ```
//! # let fbx = br#"; FBX 7.4.0 project file
//! # FBXHeaderExtension: { FBXVersion: 7400 }
//! # Objects: {
//! #  Geometry: 1, "Geometry::Quad", "Mesh" {
//! #   Vertices: *12 { a: 0,0,0, 1,0,0, 1,1,0, 0,1,0 }
//! #   PolygonVertexIndex: *4 { a: 0,1,2,-4 }
//! #  }
//! #  Model: 2, "Model::Quad", "Mesh" {}
//! # }
//! # Connections: { C: "OO",1,2
//! #  C: "OO",2,0
//! # }
//! # "#;
//! use fbx_rs::meshoptimizer_rs::{simplify, SimplifyOptions, SimplifySettings, Workspace};
//!
//! let scene = fbx_rs::load(fbx, &fbx_rs::LoadOptions::default())?;
//! let input = scene.meshes()[0].to_meshopt(&fbx_rs::MeshoptOptions::default())?;
//! assert_eq!(input.vertex_count(), 4);
//! let mut workspace = Workspace::default();
//! for part in input.parts() {
//!     let settings = SimplifySettings {
//!         target_index_count: part.index_count / 2,
//!         target_error: 0.01,
//!         options: SimplifyOptions::EMPTY,
//!     };
//!     let lod = simplify(input.part_indices(part), input.position_view(), settings, &mut workspace)?;
//!     assert!(lod.indices.len() <= part.index_count);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
use crate::mesh_data::{self, check_corners, reserve, value, MeshAttribute, MeshDataError};
use crate::{Mesh, VertexAttribute};
use meshoptimizer_rs::{Attributes, Positions};
use std::collections::HashMap;

/// Which attributes [`Mesh::to_meshopt`] emits. Each emitted attribute takes
/// part in welding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshoptOptions {
    /// Emit normals (authored or generated) when the mesh has them.
    pub normals: bool,
    /// UV set to emit (index into [`Mesh::uv_sets`]); no UVs are emitted when
    /// the mesh has no such set.
    pub uv_set: Option<usize>,
    /// Emit the tangents of the chosen UV set when the file has them.
    pub tangents: bool,
}

impl Default for MeshoptOptions {
    /// Normals, the first UV set and its tangents.
    fn default() -> Self {
        Self {
            normals: true,
            uv_set: Some(0),
            tangents: true,
        }
    }
}

/// A run of [`MeshoptMesh::indices`] that uses one material slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshoptPart {
    /// Material slot (index into [`Mesh::materials`]).
    pub material_slot: u32,
    /// The slot's material (index into [`crate::Scene::materials`]), or
    /// `None` when the mesh has no material for the slot.
    pub material: Option<usize>,
    /// First index of the run.
    pub first_index: usize,
    /// Number of indices (three per triangle).
    pub index_count: usize,
}

/// Welded vertex streams and a triangle list ready for meshoptimizer-rs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshoptMesh {
    positions: Vec<[f32; 3]>,
    normals: Option<Vec<[f32; 3]>>,
    uvs: Option<Vec<[f32; 2]>>,
    tangents: Option<Vec<[f32; 4]>>,
    source_corners: Vec<u32>,
    indices: Vec<u32>,
    parts: Vec<MeshoptPart>,
    degenerate_triangles: usize,
}

impl MeshoptMesh {
    /// Number of welded vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }
    /// Vertex positions.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }
    /// Positions as a meshoptimizer-rs view.
    pub fn position_view(&self) -> Positions<'_> {
        Positions::from_packed(&self.positions)
    }
    /// Vertex normals, when emitted.
    pub fn normals(&self) -> Option<&[[f32; 3]]> {
        self.normals.as_deref()
    }
    /// Vertex texture coordinates, when emitted.
    pub fn uvs(&self) -> Option<&[[f32; 2]]> {
        self.uvs.as_deref()
    }
    /// Vertex tangents `[x, y, z, w]`, when emitted. `w` is the bitangent
    /// sign: -1 when the file's bitangent points against
    /// `cross(normal, tangent)`, otherwise 1.
    pub fn tangents(&self) -> Option<&[[f32; 4]]> {
        self.tangents.as_deref()
    }
    /// For each vertex, the first polygon corner welded into it; use it to
    /// fetch attributes this conversion does not emit (colors, other UV sets).
    pub fn source_corners(&self) -> &[u32] {
        &self.source_corners
    }
    /// Triangle list, grouped by material slot (see [`MeshoptMesh::parts`]).
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
    /// Material runs of [`MeshoptMesh::indices`], ascending by slot. Slots
    /// without a remaining triangle have no run.
    pub fn parts(&self) -> &[MeshoptPart] {
        &self.parts
    }
    /// Indices of one part. A part from another mesh yields an empty slice.
    pub fn part_indices(&self, part: &MeshoptPart) -> &[u32] {
        part.first_index
            .checked_add(part.index_count)
            .and_then(|end| self.indices.get(part.first_index..end))
            .unwrap_or(&[])
    }
    /// Triangles dropped because their corners do not have three distinct
    /// positions.
    pub fn degenerate_triangles(&self) -> usize {
        self.degenerate_triangles
    }
    /// Normals and UVs interleaved per vertex, for attribute-aware
    /// simplification (`simplify_with_attributes`).
    ///
    /// # Errors
    /// [`MeshDataError::Allocation`] when the buffer cannot be reserved.
    pub fn attribute_stream(&self) -> Result<AttributeStream, MeshDataError> {
        let normals = self.normals.as_deref();
        let uvs = self.uvs.as_deref();
        let components = normals.map_or(0, |_| 3) + uvs.map_or(0, |_| 2);
        let count = self.positions.len();
        let mut data = Vec::new();
        reserve(
            &mut data,
            count
                .checked_mul(components)
                .ok_or(MeshDataError::TooManyVertices)?,
        )?;
        for v in 0..count {
            if let Some(n) = normals.and_then(|n| n.get(v)) {
                data.extend_from_slice(n);
            }
            if let Some(uv) = uvs.and_then(|u| u.get(v)) {
                data.extend_from_slice(uv);
            }
        }
        Ok(AttributeStream {
            data,
            components,
            count,
        })
    }
}

/// Interleaved per-vertex attributes from [`MeshoptMesh::attribute_stream`]:
/// the normal (3 floats) when emitted, then the UV (2 floats) when emitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttributeStream {
    data: Vec<f32>,
    components: usize,
    count: usize,
}

impl AttributeStream {
    /// Floats per vertex (0, 2, 3 or 5).
    pub fn components(&self) -> usize {
        self.components
    }
    /// The interleaved floats.
    pub fn data(&self) -> &[f32] {
        &self.data
    }
    /// The stream as a meshoptimizer-rs view.
    ///
    /// # Errors
    /// The view constructor's layout errors.
    pub fn view(&self) -> Result<Attributes<'_>, meshoptimizer_rs::Error> {
        Attributes::from_interleaved(&self.data, self.count, self.components, self.components, 0)
    }
}

const KEY: usize = 12;

/// Convert a per-corner value to finite binary32.
fn finite<const N: usize>(
    v: [f64; N],
    attribute: MeshAttribute,
    corner: usize,
) -> Result<[f32; N], MeshDataError> {
    let mut out = [0.0f32; N];
    for (o, x) in out.iter_mut().zip(v) {
        let y = crate::numeric::round_f32(x);
        if !y.is_finite() {
            return Err(MeshDataError::NonFinite { attribute, corner });
        }
        *o = y;
    }
    Ok(out)
}

/// The attributes the conversion reads, after option selection.
struct Sources<'a> {
    positions: &'a VertexAttribute<3>,
    normals: Option<&'a VertexAttribute<3>>,
    uvs: Option<&'a VertexAttribute<2>>,
    tangents: Option<&'a VertexAttribute<3>>,
    bitangents: Option<&'a VertexAttribute<3>>,
}

/// One corner's emitted values and its weld key.
struct Corner {
    key: [u32; KEY],
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent: [f32; 4],
}

impl Sources<'_> {
    fn corner(&self, corner: usize) -> Result<Corner, MeshDataError> {
        use MeshAttribute as A;
        let mut out = Corner {
            key: [0; KEY],
            position: finite(
                value(self.positions, corner, A::Position)?,
                A::Position,
                corner,
            )?,
            normal: [0.0; 3],
            uv: [0.0; 2],
            tangent: [0.0; 4],
        };
        let mut normal64 = None;
        if let Some(n) = self.normals {
            let v = value(n, corner, A::Normal)?;
            out.normal = finite(v, A::Normal, corner)?;
            normal64 = Some(v);
        }
        if let Some(uv) = self.uvs {
            out.uv = finite(value(uv, corner, A::Uv)?, A::Uv, corner)?;
        }
        if let Some(t) = self.tangents {
            let t = value(t, corner, A::Tangent)?;
            let [x, y, z] = finite(t, A::Tangent, corner)?;
            let mut w = 1.0;
            if let (Some(b), Some(n)) = (self.bitangents, normal64) {
                let b = value(b, corner, A::Bitangent)?;
                finite(b, A::Bitangent, corner)?;
                let c = [
                    n[1] * t[2] - n[2] * t[1],
                    n[2] * t[0] - n[0] * t[2],
                    n[0] * t[1] - n[1] * t[0],
                ];
                if c[0] * b[0] + c[1] * b[1] + c[2] * b[2] < 0.0 {
                    w = -1.0;
                }
            }
            out.tangent = [x, y, z, w];
        }
        let words = out
            .position
            .iter()
            .chain(&out.normal)
            .chain(&out.uv)
            .chain(&out.tangent);
        for (k, v) in out.key.iter_mut().zip(words) {
            *k = v.to_bits();
        }
        Ok(out)
    }
}

impl Mesh {
    /// Convert this mesh into welded meshoptimizer-rs inputs (feature
    /// `meshoptimizer`).
    ///
    /// Corners are welded when every emitted attribute is bitwise equal at
    /// 32-bit precision; the first corner of each vertex, in output
    /// triangle order, is kept in [`MeshoptMesh::source_corners`]. Triangles
    /// are grouped by the material slot of their face, keeping face order
    /// within a slot, and triangles without three distinct positions are
    /// dropped (the same rule as [`crate::MeshReport::degenerate_triangles`]).
    /// Only corners used by a triangle are read.
    ///
    /// # Errors
    /// A [`MeshDataError`] for an incomplete triangle list, a corner out of
    /// range or outside its face, a face-material list of the wrong length,
    /// attribute indices that do not cover every corner or point past the
    /// values, a NaN, infinite or binary32-overflowing value, more than
    /// `u32::MAX` vertices, or a refused allocation.
    pub fn to_meshopt(&self, options: &MeshoptOptions) -> Result<MeshoptMesh, MeshDataError> {
        mesh_data::check_triangles(self)?;
        let corners = self.num_corners();
        let uv_set = options.uv_set.and_then(|i| self.uv_sets().get(i));
        let tangents = uv_set
            .filter(|_| options.tangents)
            .and_then(|s| s.tangent());
        let src = Sources {
            positions: self.positions(),
            normals: self.normals().filter(|_| options.normals),
            uvs: uv_set.map(|s| s.uv()),
            tangents,
            bitangents: tangents.and(uv_set.and_then(|s| s.bitangent())),
        };
        check_corners(src.positions, corners, MeshAttribute::Position)?;
        if let Some(a) = src.normals {
            check_corners(a, corners, MeshAttribute::Normal)?;
        }
        if let Some(a) = src.uvs {
            check_corners(a, corners, MeshAttribute::Uv)?;
        }
        if let Some(a) = src.tangents {
            check_corners(a, corners, MeshAttribute::Tangent)?;
        }
        if let Some(a) = src.bitangents {
            check_corners(a, corners, MeshAttribute::Bitangent)?;
        }
        let faces = self.faces();
        let face_material = self.face_material();
        if !face_material.is_empty() && face_material.len() != faces.len() {
            return Err(MeshDataError::FaceMaterialCount);
        }

        // Material slot of every triangle, from the face holding its corners.
        let triangles = self.triangles();
        let count = triangles.len() / 3;
        let mut order: Vec<(u32, usize)> = Vec::new();
        reserve(&mut order, count)?;
        for (t, tri) in triangles.as_chunks::<3>().0.iter().enumerate() {
            let outside = MeshDataError::TriangleOutsideFace { triangle: t };
            let f = faces
                .partition_point(|f| f.begin <= tri[0])
                .checked_sub(1)
                .ok_or(outside)?;
            let face = faces[f];
            let end = u64::from(face.begin) + u64::from(face.count);
            if tri.iter().any(|&c| c < face.begin || u64::from(c) >= end) {
                return Err(outside);
            }
            order.push((face_material.get(f).copied().unwrap_or(0), t));
        }
        order.sort_by_key(|&(slot, _)| slot);

        let max_vertices = corners.min(triangles.len());
        let mut out = MeshoptMesh::default();
        reserve(&mut out.positions, max_vertices)?;
        reserve(&mut out.source_corners, max_vertices)?;
        reserve(&mut out.indices, triangles.len())?;
        if src.normals.is_some() {
            let mut v = Vec::new();
            reserve(&mut v, max_vertices)?;
            out.normals = Some(v);
        }
        if src.uvs.is_some() {
            let mut v = Vec::new();
            reserve(&mut v, max_vertices)?;
            out.uvs = Some(v);
        }
        if src.tangents.is_some() {
            let mut v = Vec::new();
            reserve(&mut v, max_vertices)?;
            out.tangents = Some(v);
        }
        let mut weld: HashMap<[u32; KEY], u32> = HashMap::new();
        weld.try_reserve(max_vertices)
            .map_err(|_| MeshDataError::Allocation)?;
        let mut tri_corners = [0usize; 3];
        for &(slot, t) in order.iter() {
            let tri = &triangles[t * 3..t * 3 + 3];
            for (c, &corner) in tri_corners.iter_mut().zip(tri) {
                *c = usize::try_from(corner)
                    .map_err(|_| MeshDataError::CornerOutOfRange { triangle: t })?;
            }
            let values = [
                src.corner(tri_corners[0])?,
                src.corner(tri_corners[1])?,
                src.corner(tri_corners[2])?,
            ];
            let p = values.each_ref().map(|v| v.position.map(f32::to_bits));
            if p[0] == p[1] || p[1] == p[2] || p[0] == p[2] {
                out.degenerate_triangles += 1;
                continue;
            }
            if out
                .parts
                .last()
                .is_none_or(|part| part.material_slot != slot)
            {
                out.parts
                    .try_reserve(1)
                    .map_err(|_| MeshDataError::Allocation)?;
                out.parts.push(MeshoptPart {
                    material_slot: slot,
                    material: usize::try_from(slot)
                        .ok()
                        .and_then(|s| self.materials().get(s).copied()),
                    first_index: out.indices.len(),
                    index_count: 0,
                });
            }
            for (v, &corner) in values.iter().zip(tri) {
                let next = u32::try_from(out.positions.len())
                    .map_err(|_| MeshDataError::TooManyVertices)?;
                let index = *weld.entry(v.key).or_insert(next);
                if index == next {
                    out.positions.push(v.position);
                    out.source_corners.push(corner);
                    if let Some(n) = out.normals.as_mut() {
                        n.push(v.normal);
                    }
                    if let Some(uv) = out.uvs.as_mut() {
                        uv.push(v.uv);
                    }
                    if let Some(tg) = out.tangents.as_mut() {
                        tg.push(v.tangent);
                    }
                }
                out.indices.push(index);
            }
            if let Some(part) = out.parts.last_mut() {
                part.index_count += 3;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Face, UvSet};

    fn attr<const N: usize>(values: &[f64], indices: &[u32]) -> VertexAttribute<N> {
        VertexAttribute::from_flat(values.to_vec(), indices.to_vec())
    }

    /// A unit quad as two triangles, one face, no material.
    fn quad() -> Mesh {
        Mesh {
            num_vertices: 4,
            faces: vec![Face { begin: 0, count: 4 }],
            positions: attr(
                &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
                &[0, 1, 2, 3],
            ),
            normals: Some(attr(&[0., 0., 1.], &[0, 0, 0, 0])),
            triangles: vec![0, 1, 2, 2, 3, 0],
            ..Mesh::default()
        }
    }

    #[test]
    fn welds_drops_degenerate_and_groups_by_slot() {
        let mut m = quad();
        // Face 1 repeats the quad's positions through other control points
        // and carries a degenerate triangle.
        m.num_vertices = 8;
        m.faces.push(Face { begin: 4, count: 3 });
        m.faces.push(Face { begin: 7, count: 3 });
        m.positions = attr(
            &[
                0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0.,
            ],
            &[0, 1, 2, 3, 4, 5, 2, 0, 4, 1],
        );
        m.normals = Some(attr(&[0., 0., 1.], &[0; 10]));
        m.triangles = vec![0, 1, 2, 2, 3, 0, 4, 5, 6, 7, 8, 9];
        m.face_material = vec![1, 0, 1];
        m.materials = vec![7, 9];
        let out = m.to_meshopt(&MeshoptOptions::default()).unwrap();
        assert_eq!(out.degenerate_triangles(), 1);
        assert_eq!(out.vertex_count(), 4);
        assert_eq!(out.indices().len(), 9);
        assert!(out.indices().iter().all(|&i| (i as usize) < 4));
        let parts = out.parts();
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].material_slot, parts[0].material), (0, Some(7)));
        assert_eq!((parts[0].first_index, parts[0].index_count), (0, 3));
        assert_eq!((parts[1].material_slot, parts[1].material), (1, Some(9)));
        assert_eq!((parts[1].first_index, parts[1].index_count), (3, 6));
        // Face 1 comes first in output order, so its corners own the vertices.
        assert_eq!(&out.source_corners()[..3], &[4, 5, 6]);
        assert_eq!(out.attribute_stream().unwrap().components(), 3);
    }

    #[test]
    fn tangent_sign_follows_bitangent() {
        let mut m = quad();
        m.uv_sets = vec![UvSet {
            name: String::new(),
            uv: attr(&[0., 0.], &[0; 4]),
            tangent: Some(attr(&[1., 0., 0.], &[0; 4])),
            bitangent: Some(attr(&[0., 1., 0., 0., -1., 0.], &[0, 0, 1, 1])),
        }];
        let out = m.to_meshopt(&MeshoptOptions::default()).unwrap();
        let w: Vec<f32> = out.tangents().unwrap().iter().map(|t| t[3]).collect();
        assert_eq!(w, [1.0, 1.0, -1.0, -1.0]);
        assert_eq!(out.uvs().unwrap().len(), 4);
    }

    #[test]
    fn malformed_data_is_a_typed_error() {
        use MeshAttribute as A;
        use MeshDataError as E;
        type Case = (fn(&mut Mesh), MeshDataError);
        let cases: Vec<Case> = vec![
            (|m| m.triangles.truncate(5), E::IncompleteTriangle),
            (|m| m.triangles[4] = 9, E::CornerOutOfRange { triangle: 1 }),
            (
                |m| {
                    m.faces = vec![Face { begin: 0, count: 2 }, Face { begin: 2, count: 2 }];
                },
                E::TriangleOutsideFace { triangle: 0 },
            ),
            (|m| m.face_material = vec![0, 0], E::FaceMaterialCount),
            (
                |m| m.normals = Some(attr(&[0., 0., 1.], &[0, 0, 0])),
                E::CornerCount {
                    attribute: A::Normal,
                },
            ),
            (
                |m| m.positions = attr(&[0.; 12], &[0, 1, 2, 7]),
                E::ValueOutOfRange {
                    attribute: A::Position,
                    corner: 3,
                },
            ),
            (
                |m| m.positions = attr(&[0., f64::NAN, 0., 1., 0., 0., 1., 1., 0.], &[1, 0, 2, 1]),
                E::NonFinite {
                    attribute: A::Position,
                    corner: 1,
                },
            ),
            (
                |m| m.normals = Some(attr(&[0., 0., 1e300], &[0; 4])),
                E::NonFinite {
                    attribute: A::Normal,
                    corner: 0,
                },
            ),
        ];
        for (break_mesh, expected) in cases {
            let mut m = quad();
            break_mesh(&mut m);
            assert_eq!(m.to_meshopt(&MeshoptOptions::default()), Err(expected));
        }
    }
}
