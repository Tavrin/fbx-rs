//! Checked access to converted mesh data, shared by the import report and the
//! optional meshoptimizer path. Every inconsistency becomes a
//! [`MeshDataError`] instead of a panic.
use crate::{Mesh, VertexAttribute};
use std::collections::HashMap;

/// A per-corner mesh attribute named by a [`MeshDataError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MeshAttribute {
    /// [`Mesh::positions`].
    Position,
    /// [`Mesh::normals`].
    Normal,
    /// [`crate::UvSet::uv`].
    Uv,
    /// [`crate::UvSet::tangent`].
    Tangent,
    /// [`crate::UvSet::bitangent`].
    Bitangent,
}

impl MeshAttribute {
    fn label(self) -> &'static str {
        match self {
            MeshAttribute::Position => "position",
            MeshAttribute::Normal => "normal",
            MeshAttribute::Uv => "UV",
            MeshAttribute::Tangent => "tangent",
            MeshAttribute::Bitangent => "bitangent",
        }
    }
}

/// Inconsistent or unusable mesh data found while reading a converted mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeshDataError {
    /// The triangle index buffer length is not a multiple of three.
    IncompleteTriangle,
    /// A triangle refers to a corner past [`Mesh::num_corners`].
    CornerOutOfRange {
        /// Triangle index.
        triangle: usize,
    },
    /// The corners of a triangle do not all belong to one face.
    TriangleOutsideFace {
        /// Triangle index.
        triangle: usize,
    },
    /// [`Mesh::face_material`] has neither zero entries nor one per face.
    FaceMaterialCount,
    /// An attribute does not have one index per polygon corner.
    CornerCount {
        /// The attribute.
        attribute: MeshAttribute,
    },
    /// A corner's index refers past the attribute's values.
    ValueOutOfRange {
        /// The attribute.
        attribute: MeshAttribute,
        /// Polygon corner.
        corner: usize,
    },
    /// A value is NaN or infinite, or overflows 32-bit floats.
    NonFinite {
        /// The attribute.
        attribute: MeshAttribute,
        /// Polygon corner.
        corner: usize,
    },
    /// More vertices than 32-bit indices can address.
    TooManyVertices,
    /// An allocator refused a fallible reservation.
    Allocation,
}

impl std::fmt::Display for MeshDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            MeshDataError::IncompleteTriangle => {
                f.write_str("triangle index count is not a multiple of three")
            }
            MeshDataError::CornerOutOfRange { triangle } => {
                write!(f, "triangle {triangle} refers to a corner out of range")
            }
            MeshDataError::TriangleOutsideFace { triangle } => {
                write!(f, "triangle {triangle} spans more than one face")
            }
            MeshDataError::FaceMaterialCount => {
                f.write_str("face material count does not match the face count")
            }
            MeshDataError::CornerCount { attribute } => {
                write!(f, "{} indices do not cover every corner", attribute.label())
            }
            MeshDataError::ValueOutOfRange { attribute, corner } => write!(
                f,
                "{} index of corner {corner} is out of range",
                attribute.label()
            ),
            MeshDataError::NonFinite { attribute, corner } => write!(
                f,
                "{} of corner {corner} is not a finite 32-bit value",
                attribute.label()
            ),
            MeshDataError::TooManyVertices => f.write_str("too many vertices for 32-bit indices"),
            MeshDataError::Allocation => f.write_str("allocation refused"),
        }
    }
}

impl std::error::Error for MeshDataError {}

pub(crate) fn reserve<T>(v: &mut Vec<T>, n: usize) -> Result<(), MeshDataError> {
    v.try_reserve(n).map_err(|_| MeshDataError::Allocation)
}

/// Bits of `v` rounded to binary32: the identity used for welding and for
/// position comparisons.
pub(crate) fn f32_bits(v: f64) -> u32 {
    crate::numeric::round_f32(v).to_bits()
}

/// Check that `attr` has one index per corner.
#[cfg(feature = "meshoptimizer")]
pub(crate) fn check_corners<const N: usize>(
    attr: &VertexAttribute<N>,
    corners: usize,
    attribute: MeshAttribute,
) -> Result<(), MeshDataError> {
    if attr.indices().len() == corners {
        Ok(())
    } else {
        Err(MeshDataError::CornerCount { attribute })
    }
}

/// Index into `attr.values()` of polygon corner `corner`.
pub(crate) fn value_index<const N: usize>(
    attr: &VertexAttribute<N>,
    corner: usize,
    attribute: MeshAttribute,
) -> Result<usize, MeshDataError> {
    let out_of_range = MeshDataError::ValueOutOfRange { attribute, corner };
    let i = *attr.indices().get(corner).ok_or(out_of_range)?;
    let i = usize::try_from(i).map_err(|_| out_of_range)?;
    if i < attr.values().len() {
        Ok(i)
    } else {
        Err(out_of_range)
    }
}

/// Value of polygon corner `corner`.
#[cfg(feature = "meshoptimizer")]
pub(crate) fn value<const N: usize>(
    attr: &VertexAttribute<N>,
    corner: usize,
    attribute: MeshAttribute,
) -> Result<[f64; N], MeshDataError> {
    Ok(attr.values()[value_index(attr, corner, attribute)?])
}

/// Check the triangle buffer: whole triangles, every corner in range.
pub(crate) fn check_triangles(mesh: &Mesh) -> Result<(), MeshDataError> {
    let triangles = mesh.triangles();
    if !triangles.len().is_multiple_of(3) {
        return Err(MeshDataError::IncompleteTriangle);
    }
    let corners = mesh.num_corners();
    for (t, tri) in triangles.as_chunks::<3>().0.iter().enumerate() {
        for &c in tri {
            if usize::try_from(c).ok().is_none_or(|c| c >= corners) {
                return Err(MeshDataError::CornerOutOfRange { triangle: t });
            }
        }
    }
    Ok(())
}

/// A dense identifier per distinct position value (compared as binary32
/// bits), indexed like [`VertexAttribute::values`] of [`Mesh::positions`].
pub(crate) fn position_ids(mesh: &Mesh) -> Result<Vec<u32>, MeshDataError> {
    let values = mesh.positions().values();
    let mut ids = Vec::new();
    reserve(&mut ids, values.len())?;
    let mut seen: HashMap<[u32; 3], u32> = HashMap::new();
    seen.try_reserve(values.len())
        .map_err(|_| MeshDataError::Allocation)?;
    for p in values {
        let key = p.map(f32_bits);
        let next = u32::try_from(seen.len()).map_err(|_| MeshDataError::TooManyVertices)?;
        ids.push(*seen.entry(key).or_insert(next));
    }
    Ok(ids)
}

/// Position identifiers of the three corners of each triangle. Requires
/// [`check_triangles`].
pub(crate) fn triangle_positions<'a>(
    mesh: &'a Mesh,
    ids: &'a [u32],
) -> impl Iterator<Item = Result<[u32; 3], MeshDataError>> + 'a {
    mesh.triangles()
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(move |(triangle, tri)| {
            let mut out = [0; 3];
            for (slot, &c) in out.iter_mut().zip(tri) {
                let corner =
                    usize::try_from(c).map_err(|_| MeshDataError::CornerOutOfRange { triangle })?;
                let i = value_index(mesh.positions(), corner, MeshAttribute::Position)?;
                *slot = ids[i];
            }
            Ok(out)
        })
}

/// A triangle whose corners do not have three distinct positions has zero
/// area; the meshoptimizer path drops it and the report counts it.
pub(crate) fn is_degenerate(p: [u32; 3]) -> bool {
    p[0] == p[1] || p[1] == p[2] || p[0] == p[2]
}
