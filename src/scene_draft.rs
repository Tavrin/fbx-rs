//! A bounded composition of the currently implemented scene components.
//! This is an incomplete draft: it does not load material, camera or light data.
use crate::{scene, scene_mesh, scene_transform, syntax};
use std::{borrow::Cow, mem::size_of};

#[derive(Debug, Clone)]
pub struct Limits {
    pub index: scene::Limits,
    pub mesh: scene_mesh::Limits,
    pub max_allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            index: Default::default(),
            mesh: Default::default(),
            max_allocation: 512 << 20,
        }
    }
}
#[derive(Debug)]
pub struct Draft<'a> {
    pub index: scene::SceneIndex<'a>,
    pub transforms: scene_transform::AuthoredTransforms,
    pub meshes: scene_mesh::Meshes<'a>,
    /// Accounted DOM and retained draft vector capacities, not allocator overhead.
    pub retained_bytes: usize,
}
fn add(total: &mut usize, count: usize, width: usize) -> Result<(), scene::Error> {
    let bytes = count
        .checked_mul(width)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    *total = total
        .checked_add(bytes)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    Ok(())
}
/// Charge the already parsed DOM, including owned array and string capacities.
fn dom_bytes(doc: &syntax::Document<'_>) -> Result<usize, scene::Error> {
    let mut bytes = 0;
    add(
        &mut bytes,
        doc.nodes.capacity(),
        size_of::<syntax::Node<'_>>(),
    )?;
    add(
        &mut bytes,
        doc.properties.capacity(),
        size_of::<syntax::Property<'_>>(),
    )?;
    for property in &doc.properties {
        match property {
            syntax::Property::String(Cow::Owned(v)) => add(&mut bytes, v.capacity(), 1)?,
            syntax::Property::Array {
                bytes: Cow::Owned(v),
                ..
            } => add(&mut bytes, v.capacity(), 1)?,
            syntax::Property::AsciiArray(v) => add(
                &mut bytes,
                v.capacity(),
                size_of::<syntax::AsciiValue<'_>>(),
            )?,
            _ => (),
        }
    }
    Ok(bytes)
}
/// Build only the currently implemented components under one aggregate budget.
/// The caller's input bytes and allocator overhead are not included.
pub fn build<'a>(
    doc: &'a syntax::Document<'a>,
    limits: &Limits,
) -> Result<Draft<'a>, scene::Error> {
    let mut retained = dom_bytes(doc)?;
    let remaining = limits
        .max_allocation
        .checked_sub(retained)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    let mut index_limits = limits.index.clone();
    index_limits.max_allocation = index_limits.max_allocation.min(remaining);
    let index = scene::index(doc, &index_limits)?;
    add(
        &mut retained,
        index.objects.capacity(),
        size_of::<scene::Object>(),
    )?;
    add(
        &mut retained,
        index.connections.capacity(),
        size_of::<scene::Connection<'_>>(),
    )?;
    add(
        &mut retained,
        index.unsupported.capacity(),
        size_of::<scene::Unsupported>(),
    )?;
    let remaining = limits
        .max_allocation
        .checked_sub(retained)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    let transforms = scene_transform::authored(doc, &index, remaining)?;
    add(
        &mut retained,
        transforms.models.capacity(),
        size_of::<Option<scene_transform::Transform>>(),
    )?;
    let remaining = limits
        .max_allocation
        .checked_sub(retained)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    let mut mesh_limits = limits.mesh.clone();
    mesh_limits.max_allocation = mesh_limits.max_allocation.min(remaining);
    let meshes = scene_mesh::extract(doc, &index, &mesh_limits)?;
    add(
        &mut retained,
        meshes.meshes.capacity(),
        size_of::<scene_mesh::Mesh<'_>>(),
    )?;
    add(
        &mut retained,
        meshes.layers.capacity(),
        size_of::<scene_mesh::Layer<'_>>(),
    )?;
    if retained > limits.max_allocation {
        return Err(scene::Error::LimitExceeded("allocation"));
    }
    Ok(Draft {
        index,
        transforms,
        meshes,
        retained_bytes: retained,
    })
}

/// Draft including selected static material and node-attribute properties.
/// Full shader projection and ufbx conversion are still incomplete.
#[derive(Debug)]
pub struct StaticDraft<'a> {
    pub core: Draft<'a>,
    pub surfaces: crate::scene_surface::Surfaces<'a>,
    pub retained_bytes: usize,
}
/// Compose all implemented data under the same DOM-plus-conversion budget.
pub fn build_static<'a>(
    doc: &'a syntax::Document<'a>,
    encoding: crate::scene_surface::Encoding,
    limits: &Limits,
    surface_limits: &crate::scene_surface::Limits,
) -> Result<StaticDraft<'a>, scene::Error> {
    let core = build(doc, limits)?;
    let remaining = limits
        .max_allocation
        .checked_sub(core.retained_bytes)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    let mut surface_limits = surface_limits.clone();
    surface_limits.max_allocation = surface_limits.max_allocation.min(remaining);
    let surfaces = crate::scene_surface::extract(doc, &core.index, encoding, &surface_limits)?;
    let retained_bytes = core
        .retained_bytes
        .checked_add(surfaces.retained_bytes)
        .ok_or(scene::Error::LimitExceeded("allocation"))?;
    if retained_bytes > limits.max_allocation {
        return Err(scene::Error::LimitExceeded("allocation"));
    }
    Ok(StaticDraft {
        core,
        surfaces,
        retained_bytes,
    })
}
