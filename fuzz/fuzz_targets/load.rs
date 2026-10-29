#![no_main]
//! Public API robustness: `fbx_rs::load_with_limits` under small limits, then
//! every cross-reference the returned `Scene` exposes must be in range.
use fbx_rs::{load_with_limits, Limits, LoadOptions, MaterialMap, Scene};
use libfuzzer_sys::fuzz_target;

fn check_map(scene: &Scene, map: &MaterialMap) {
    if let Some(t) = map.texture {
        assert!(t < scene.textures().len());
    }
}

fn walk(scene: &Scene) {
    let nodes = scene.nodes().len();
    for node in scene.nodes() {
        assert!(node.parent().is_none_or(|p| p < nodes));
        assert!(node.children().iter().all(|&c| c < nodes));
        assert!(node.mesh().is_none_or(|m| m < scene.meshes().len()));
        assert!(node.camera().is_none_or(|c| c < scene.cameras().len()));
        assert!(node.light().is_none_or(|l| l < scene.lights().len()));
        assert!(node
            .materials()
            .iter()
            .all(|&m| m < scene.materials().len()));
    }
    for mesh in scene.meshes() {
        assert!(mesh
            .materials()
            .iter()
            .all(|&m| m < scene.materials().len()));
        assert!(mesh.instances().iter().all(|&n| n < nodes));
        let positions = mesh.positions();
        for corner in 0..positions.indices().len() {
            assert!(positions.get(corner).is_some());
        }
        for face in mesh.faces() {
            std::hint::black_box(face);
        }
        std::hint::black_box((mesh.triangles(), mesh.parts(), mesh.uv_sets()));
    }
    for material in scene.materials() {
        for (_, map) in material.fbx_maps().chain(material.pbr_maps()) {
            check_map(scene, map);
        }
    }
    std::hint::black_box((scene.settings(), scene.unsupported(), scene.warnings()));
}

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_input_bytes: 1 << 20,
        max_depth: 16,
        max_nodes: 256,
        max_properties: 1024,
        max_array_elements: 4096,
        max_decompressed_bytes: 32768,
        max_string_length: 8192,
        max_objects: 256,
        max_connections: 256,
        max_allocation: 131072,
    };
    if let Ok(scene) = load_with_limits(data, &LoadOptions::default(), &limits) {
        walk(&scene);
    }
});
