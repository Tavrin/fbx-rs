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
            .skin_deformers()
            .iter()
            .all(|&i| i < scene.skin_deformers().len()));
        assert!(mesh
            .blend_deformers()
            .iter()
            .all(|&i| i < scene.blend_deformers().len()));
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
    for node in scene.nodes() {
        assert!(node.bone().is_none_or(|i| i < scene.bones().len()));
        assert!(node.bind_pose().is_none_or(|i| i < scene.poses().len()));
    }
    for bone in scene.bones() {
        assert!(bone.instances.iter().all(|&i| i < nodes));
    }
    for skin in scene.skin_deformers() {
        assert!(skin
            .clusters
            .iter()
            .all(|&i| i < scene.skin_clusters().len()));
        assert_eq!(skin.dq_vertices.len(), skin.dq_weights.len());
        assert!(skin
            .dq_vertices
            .iter()
            .all(|&i| (i as usize) < skin.vertices.len()));
        for v in &skin.vertices {
            assert!(v.weight_begin <= skin.weights.len());
            assert!(v.num_weights <= skin.weights.len() - v.weight_begin);
        }
        assert!(skin
            .weights
            .iter()
            .all(|w| w.cluster_index < skin.clusters.len()));
    }
    for cluster in scene.skin_clusters() {
        assert!(cluster.bone_node.is_none_or(|i| i < nodes));
        assert_eq!(cluster.vertices.len(), cluster.weights.len());
    }
    for blend in scene.blend_deformers() {
        assert!(blend
            .channels
            .iter()
            .all(|&i| i < scene.blend_channels().len()));
    }
    for channel in scene.blend_channels() {
        assert!(channel
            .target_shape
            .is_none_or(|i| i < scene.blend_shapes().len()));
        assert!(channel
            .keyframes
            .iter()
            .all(|k| k.shape < scene.blend_shapes().len()));
    }
    for shape in scene.blend_shapes() {
        assert_eq!(shape.offset_vertices.len(), shape.position_offsets.len());
        assert!(
            shape.normal_offsets.is_empty()
                || shape.normal_offsets.len() == shape.offset_vertices.len()
        );
        assert!(shape.offset_vertices.windows(2).all(|p| p[0] <= p[1]));
    }
    for pose in scene.poses() {
        assert!(pose.bone_poses.iter().all(|b| b.bone_node < nodes));
    }
    for material in scene.materials() {
        for (_, map) in material.fbx_maps().chain(material.pbr_maps()) {
            check_map(scene, map);
        }
    }
    for stack in scene.animation_stacks() {assert!(stack.layers.iter().all(|&i|i<scene.animation_layers().len()));}
    for layer in scene.animation_layers() {assert!(layer.values.iter().all(|&i|i<scene.animation_values().len()));for binding in &layer.bindings {assert!(binding.value<scene.animation_values().len());}}
    for value in scene.animation_values() {assert!(value.curves.iter().flatten().all(|&i|i<scene.animation_curves().len()));}
    for curve in scene.animation_curves() {assert!(curve.keys.windows(2).all(|w|w[0].time<=w[1].time));}
    let pose=scene.evaluate(None,0.5).unwrap();
    assert_eq!(pose.nodes.len(),scene.nodes().len());
    assert_eq!(pose.blend_weights.len(),scene.blend_channels().len());
    assert_eq!(pose.skin_matrices.len(),scene.skin_clusters().len());
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
        max_animation_keys: 4096,
        max_connections: 256,
        max_allocation: 131072,
    };
    if let Ok(scene) = load_with_limits(data, &LoadOptions::default(), &limits) {
        walk(&scene);
    }
});
