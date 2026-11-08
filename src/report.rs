//! Per-asset import report: the conversions the loader applied, the helper
//! nodes and transforms it baked, and what it found in each mesh.
use crate::mesh_data::{self, reserve, MeshDataError};
use crate::{Axis, CoordinateAxes, Error, Mesh, Scene};

/// How the file's coordinate system and unit were converted to the loaded
/// scene's.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct SpaceReport {
    /// Axes declared by the file's global settings, or `None` when they are
    /// missing or invalid; no axis conversion is applied then.
    pub source_axes: Option<CoordinateAxes>,
    /// Axes of the loaded scene.
    pub target_axes: CoordinateAxes,
    /// Meters per unit declared by the file (`UnitScaleFactor`; FBX
    /// defaults to centimeters).
    pub source_unit_meters: f64,
    /// Meters per unit of the loaded scene.
    pub target_unit_meters: f64,
    /// Uniform scale baked into vertex positions and node translations.
    pub geometry_scale: f64,
    /// Rotation `[x, y, z, w]` applied to the roots of the scene to reach the
    /// target axes.
    pub root_rotation: [f64; 4],
    /// Axis mirrored to turn a left-handed source right-handed; the mesh
    /// windings are reversed with it (see [`crate::Mesh::reversed_winding`]).
    pub mirror_axis: Option<Axis>,
}

impl SpaceReport {
    /// True when the source axes differ from the target axes.
    pub fn axes_converted(&self) -> bool {
        self.source_axes.is_some_and(|a| a != self.target_axes)
    }
    /// True when geometry was scaled to the target unit.
    pub fn unit_converted(&self) -> bool {
        self.geometry_scale != 1.0
    }
}

/// Conversion facts recorded at load time for [`Scene::import_report`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoadBakes {
    pub(crate) space: SpaceReport,
    /// (owner node, generated helper node), as indices into [`Scene::nodes`].
    pub(crate) scale_helpers: Vec<(usize, usize)>,
    /// Nodes whose geometric transform was baked into their mesh.
    pub(crate) geometry_transforms: Vec<usize>,
}

/// A generated scale helper node.
///
/// When a child does not inherit its parent's scale, the loader inserts a
/// helper child under the parent that carries the parent's scale, as ufbx
/// does, so that every world matrix stays exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScaleHelper {
    /// The node whose scale the helper carries (index into [`Scene::nodes`]).
    pub node: usize,
    /// The generated helper (index into [`Scene::nodes`]).
    pub helper: usize,
}

/// What the loader found in one mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshReport {
    /// Index into [`Scene::meshes`].
    pub mesh: usize,
    /// Number of polygons.
    pub faces: usize,
    /// Faces with fewer than three corners (points and lines); they produce
    /// no triangles.
    pub skipped_faces: usize,
    /// Number of triangles in [`crate::Mesh::triangles`].
    pub triangles: usize,
    /// Triangles without three distinct positions (compared at 32-bit float
    /// precision). They are kept in [`crate::Mesh::triangles`] and dropped
    /// by the meshoptimizer path.
    pub degenerate_triangles: usize,
    /// Edges used by exactly one non-degenerate triangle. Edges join
    /// positions, so coincident control points count as one.
    pub boundary_edges: usize,
    /// Edges used by more than two non-degenerate triangles.
    pub non_manifold_edges: usize,
    /// True when the file has no normals and they were generated.
    pub generated_normals: bool,
    /// True when the mesh has no UV set.
    pub missing_uvs: bool,
    /// True when the axis conversion mirrored the mesh.
    pub reversed_winding: bool,
}

/// One texture attached to a material.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextureLink {
    /// Index into [`Scene::materials`].
    pub material: usize,
    /// Material property the texture is attached to, as authored.
    pub property: String,
    /// Index into [`Scene::textures`].
    pub texture: usize,
    /// True when the file embeds the texture content.
    pub embedded: bool,
    /// True when the texture records a file name.
    pub has_filename: bool,
    /// True for a layered texture (its layers are separate textures).
    pub layered: bool,
}

impl TextureLink {
    /// True when the link has neither embedded content, a file name nor
    /// layers: nothing to load.
    pub fn is_unresolved(&self) -> bool {
        !self.embedded && !self.has_filename && !self.layered
    }
}

/// The import report of a loaded scene; see [`Scene::import_report`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ImportReport {
    /// Axis and unit conversion.
    pub space: SpaceReport,
    /// Generated scale helper nodes, in the order they were generated.
    pub scale_helpers: Vec<ScaleHelper>,
    /// Nodes whose geometric (pivot-only) transform was baked into the
    /// vertex data of their mesh (indices into [`Scene::nodes`]).
    pub baked_geometry_transforms: Vec<usize>,
    /// One entry per mesh, in [`Scene::meshes`] order.
    pub meshes: Vec<MeshReport>,
    /// Material texture links, by material then attachment order.
    pub textures: Vec<TextureLink>,
}

pub(crate) fn build(scene: &Scene) -> Result<ImportReport, Error> {
    let bakes = &scene.bakes;
    let mut meshes = Vec::new();
    reserve(&mut meshes, scene.meshes().len()).map_err(Error::MeshData)?;
    for (index, mesh) in scene.meshes().iter().enumerate() {
        meshes.push(mesh_report(index, mesh).map_err(Error::MeshData)?);
    }
    let mut textures = Vec::new();
    for (material, m) in scene.materials().iter().enumerate() {
        for link in m.textures() {
            let texture = scene
                .textures()
                .get(link.texture)
                .ok_or(Error::InvalidData("import report: texture index"))?;
            reserve(&mut textures, 1).map_err(Error::MeshData)?;
            textures.push(TextureLink {
                material,
                property: String::from_utf8_lossy(&link.material_prop).into_owned(),
                texture: link.texture,
                embedded: !texture.content().is_empty(),
                has_filename: !(texture.filename().is_empty()
                    && texture.relative_filename().is_empty()
                    && texture.absolute_filename().is_empty()),
                layered: texture.is_layered(),
            });
        }
    }
    let mut scale_helpers = Vec::new();
    reserve(&mut scale_helpers, bakes.scale_helpers.len()).map_err(Error::MeshData)?;
    scale_helpers.extend(
        bakes
            .scale_helpers
            .iter()
            .map(|&(node, helper)| ScaleHelper { node, helper }),
    );
    let mut baked_geometry_transforms = Vec::new();
    reserve(
        &mut baked_geometry_transforms,
        bakes.geometry_transforms.len(),
    )
    .map_err(Error::MeshData)?;
    baked_geometry_transforms.extend_from_slice(&bakes.geometry_transforms);
    Ok(ImportReport {
        space: bakes.space,
        scale_helpers,
        baked_geometry_transforms,
        meshes,
        textures,
    })
}

fn mesh_report(index: usize, mesh: &Mesh) -> Result<MeshReport, MeshDataError> {
    mesh_data::check_triangles(mesh)?;
    let ids = mesh_data::position_ids(mesh)?;
    let mut edges: Vec<(u32, u32)> = Vec::new();
    reserve(&mut edges, mesh.triangles().len())?;
    let mut degenerate_triangles = 0;
    for tri in mesh_data::triangle_positions(mesh, &ids) {
        let p = tri?;
        if mesh_data::is_degenerate(p) {
            degenerate_triangles += 1;
            continue;
        }
        for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
            edges.push((a.min(b), a.max(b)));
        }
    }
    edges.sort_unstable();
    let (mut boundary_edges, mut non_manifold_edges) = (0, 0);
    for run in edges.chunk_by(|a, b| a == b) {
        match run.len() {
            1 => boundary_edges += 1,
            2 => {}
            _ => non_manifold_edges += 1,
        }
    }
    Ok(MeshReport {
        mesh: index,
        faces: mesh.faces().len(),
        skipped_faces: mesh.faces().iter().filter(|f| f.count < 3).count(),
        triangles: mesh.triangles().len() / 3,
        degenerate_triangles,
        boundary_edges,
        non_manifold_edges,
        generated_normals: mesh.generated_normals() || mesh.normals().is_none(),
        missing_uvs: mesh.uv_sets().is_empty(),
        reversed_winding: mesh.reversed_winding(),
    })
}
