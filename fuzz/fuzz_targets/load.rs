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
    for stack in scene.animation_stacks() {
        assert!(stack
            .layers
            .iter()
            .all(|&i| i < scene.animation_layers().len()));
    }
    for layer in scene.animation_layers() {
        assert!(layer
            .values
            .iter()
            .all(|&i| i < scene.animation_values().len()));
        for binding in &layer.bindings {
            assert!(binding.value < scene.animation_values().len());
        }
    }
    for value in scene.animation_values() {
        assert!(value
            .curves
            .iter()
            .flatten()
            .all(|&i| i < scene.animation_curves().len()));
    }
    for curve in scene.animation_curves() {
        assert!(curve.keys.windows(2).all(|w| w[0].time <= w[1].time));
        // Key times, neighbours and far outside the keys, so interpolation and
        // every extrapolation mode run; only nonfinite times are errors.
        let mut times = vec![0.0, -1.0e9, 1.0e9, f64::MAX, f64::MIN];
        if let (Some(first), Some(last)) = (curve.keys.first(), curve.keys.last()) {
            times.extend([first.time - 1.0, last.time + 1.0, last.time * 3.0]);
            times.extend(curve.keys.iter().take(8).flat_map(|k| [k.time, k.time + 1e-4]));
        }
        for t in times.into_iter().filter(|t| t.is_finite()) {
            assert!(curve.evaluate(t, 0.0).is_ok());
        }
        assert!(curve.evaluate(f64::NAN, 0.0).is_err());
        assert!(curve.evaluate(f64::INFINITY, 0.0).is_err());
    }
    if let Some(i) = scene.default_animation_stack() {
        assert!(i < scene.animation_stacks().len());
    }
    // Every stack, through its own time range, including the stored bounds.
    let stacks = scene.animation_stacks().len();
    for (index, stack) in scene.animation_stacks().iter().enumerate().take(8) {
        for t in [stack.time_begin, stack.time_end, 0.0, 1.0e6] {
            if t.is_finite() {
                let pose = scene.evaluate(Some(index), t).unwrap();
                assert_eq!(pose.nodes.len(), scene.nodes().len());
                assert_eq!(pose.blend_weights.len(), scene.blend_channels().len());
                assert_eq!(pose.skin_matrices.len(), scene.skin_clusters().len());
            }
        }
    }
    assert!(scene.evaluate(Some(stacks), 0.0).is_err());
    assert!(scene.evaluate(None, f64::NAN).is_err());
    let nodes_len = scene.nodes().len();
    if nodes_len > 0 {
        assert!(scene.evaluate_transform(None, nodes_len - 1, 0.25).is_ok());
    }
    assert!(scene.evaluate_transform(None, nodes_len, 0.25).is_err());
    // Bindings and source-id property lookups, including generated helpers.
    for (index, layer) in scene.animation_layers().iter().enumerate().take(4) {
        let stack = scene
            .animation_stacks()
            .iter()
            .position(|s| s.layers.contains(&index));
        for binding in layer.bindings.iter().take(16) {
            assert!(scene.evaluate_binding(stack, binding, 0.5).is_ok());
            assert!(scene
                .evaluate_binding(stack, binding, f64::INFINITY)
                .is_err());
            if let Ok(p) = scene.evaluate_property(stack, binding.source_id, &binding.property, 0.5)
            {
                std::hint::black_box(p);
            }
        }
    }
    let pose = scene.evaluate(None, 0.5).unwrap();
    assert_eq!(pose.nodes.len(), scene.nodes().len());
    assert_eq!(pose.blend_weights.len(), scene.blend_channels().len());
    assert_eq!(pose.skin_matrices.len(), scene.skin_clusters().len());
    std::hint::black_box((scene.settings(), scene.unsupported(), scene.warnings()));
}

fuzz_target!(|data: &[u8]| {
    // Small limits, just large enough for every supported corpus file (skins,
    // blend shapes and animation included) to load, so the seeds reach those paths.
    let limits = Limits {
        max_input_bytes: 1 << 20,
        max_depth: 16,
        max_nodes: 2048,
        max_properties: 8192,
        max_array_elements: 65536,
        max_decompressed_bytes: 262144,
        max_string_length: 8192,
        max_objects: 1024,
        max_animation_keys: 4096,
        max_connections: 2048,
        max_allocation: 1 << 20,
        max_deformer_work: 1 << 20,
        max_animation_work: 1 << 20,
    };
    if let Ok(scene) = load_with_limits(data, &LoadOptions::default(), &limits) {
        walk(&scene);
    }
});
