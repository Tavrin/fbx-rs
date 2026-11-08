//! Load an FBX file, print its import report, and build a LOD chain for every
//! mesh with meshoptimizer-rs attribute-aware simplification.
//!
//! ```text
//! cargo run --example meshopt_lod --features meshoptimizer -- model.fbx
//! ```
use fbx_rs::meshoptimizer_rs::{
    optimize_vertex_cache, simplify_scale, simplify_with_attributes, SimplifyOptions,
    SimplifySettings, Workspace,
};
use fbx_rs::{LoadOptions, MeshoptOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: meshopt_lod <file.fbx>")?;
    let scene = fbx_rs::load(&std::fs::read(path)?, &LoadOptions::default())?;

    let report = scene.import_report()?;
    let space = &report.space;
    println!(
        "space: axes {:?} -> {:?}, unit {} m -> {} m, geometry scale {}, mirror {:?}",
        space.source_axes,
        space.target_axes,
        space.source_unit_meters,
        space.target_unit_meters,
        space.geometry_scale,
        space.mirror_axis
    );
    println!(
        "bakes: {} scale helpers, {} geometric transforms",
        report.scale_helpers.len(),
        report.baked_geometry_transforms.len()
    );
    let unresolved = report.textures.iter().filter(|t| t.is_unresolved()).count();
    println!(
        "textures: {} links, {} unresolved",
        report.textures.len(),
        unresolved
    );

    let mut workspace = Workspace::default();
    for (mesh, info) in scene.meshes().iter().zip(&report.meshes) {
        println!(
            "mesh {:?}: {} triangles, {} degenerate, {} boundary and {} non-manifold edges{}{}",
            mesh.name(),
            info.triangles,
            info.degenerate_triangles,
            info.boundary_edges,
            info.non_manifold_edges,
            if info.generated_normals {
                ", generated normals"
            } else {
                ""
            },
            if info.missing_uvs { ", no UVs" } else { "" },
        );
        let input = mesh.to_meshopt(&MeshoptOptions::default())?;
        if input.vertex_count() == 0 {
            continue;
        }
        let attributes = input.attribute_stream()?;
        // Normals matter less than UV seams; weights are per component.
        let weights: Vec<f32> = match attributes.components() {
            5 => vec![0.5, 0.5, 0.5, 1.0, 1.0],
            3 => vec![0.5; 3],
            n => vec![1.0; n],
        };
        let scale = simplify_scale(input.position_view())?;
        for part in input.parts() {
            let mut current = input.part_indices(part).to_vec();
            print!("  slot {}: {}", part.material_slot, current.len() / 3);
            for divisor in [2, 4] {
                let target = part.index_count / divisor / 3 * 3;
                let settings = SimplifySettings {
                    target_index_count: target.min(current.len()),
                    target_error: 0.01,
                    options: SimplifyOptions::EMPTY,
                };
                let lod = simplify_with_attributes(
                    &current,
                    input.position_view(),
                    attributes.view()?,
                    &weights,
                    None,
                    settings,
                    &mut workspace,
                )?;
                current =
                    optimize_vertex_cache(&lod.indices, input.vertex_count(), &mut workspace)?;
                print!(" -> {} (error {:.4})", current.len() / 3, lod.error * scale);
            }
            println!();
        }
    }
    Ok(())
}
