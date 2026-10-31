#![no_main]
//! Public API robustness: `fbx_rs::load_with_limits` under small limits, then
//! every cross-reference the returned `Scene` exposes must be in range.
use fbx_rs::{load_with_limits, BakeOptions, Limits, LoadOptions, MaterialMap, Scene};
use libfuzzer_sys::fuzz_target;

/// Documented list order: a parent precedes its children and the hierarchy
/// depth never decreases along `Scene::nodes`.
fn check_order(scene: &Scene) {
    let nodes = scene.nodes();
    let mut depth = vec![0usize; nodes.len()];
    let mut last = 0usize;
    for (i, node) in nodes.iter().enumerate() {
        if let Some(p) = node.parent() {
            assert!(p < i, "parent {p} does not precede child {i}");
            depth[i] = depth[p] + 1;
        }
        assert!(depth[i] >= last, "depth decreases at node {i}");
        last = depth[i];
    }
}

/// Material and texture API: every index in range, lookups agree with the
/// ordered iterators.
fn check_materials(scene: &Scene) {
    let textures = scene.textures().len();
    for material in scene.materials() {
        for (name, feature) in material.features() {
            assert_eq!(material.feature(name), Some(feature));
        }
        for window in material.source_properties().windows(2) {
            assert!(window[0].name < window[1].name);
        }
        for p in material.source_properties().iter().take(32) {
            assert_eq!(material.source_property(&p.name).map(|q| &q.name), Some(&p.name));
        }
        assert!(material.textures().iter().all(|t| t.texture < textures));
        for (name, map) in material.fbx_maps() {
            // Pointer identity: values may hold NaN, which never compares equal.
            assert!(material.fbx_map(name).is_some_and(|m| std::ptr::eq(m, map)));
        }
        for (name, map) in material.pbr_maps() {
            assert!(material.pbr_map(name).is_some_and(|m| std::ptr::eq(m, map)));
        }
        std::hint::black_box((material.name(), material.source_id(), material.shading_model()));
    }
    for texture in scene.textures() {
        assert!(texture.layers().iter().all(|l| l.texture < textures));
        assert!(texture.file_textures().iter().all(|&t| t < textures));
        if !texture.is_layered() {
            assert!(texture.layers().is_empty());
        }
        std::hint::black_box((texture.filename(), texture.name(), texture.source_id()));
    }
}

/// Bake every stack under fixed option sets plus one derived from the input.
fn check_bake(scene: &Scene, data: &[u8]) {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let pick = |shift: u32, count: u64| usize::try_from((hash >> shift) % count).unwrap_or(0);
    let mut sets = vec![BakeOptions::default()];
    let mut raw = BakeOptions::default();
    raw.trim_start_time = true;
    raw.key_reduction_enabled = false;
    sets.push(raw);
    let mut rot = BakeOptions::default();
    rot.trim_start_time = true;
    rot.key_reduction_rotation = true;
    rot.bake_transform_properties = true;
    sets.push(rot);
    let mut random = BakeOptions::default();
    random.trim_start_time = hash & 1 == 1;
    random.resample_rate = [1.0, 7.5, 30.0, 240.0, 1.0e4][pick(1, 5)];
    random.minimum_sample_rate = [0.0, 1.0, 19.5, 1.0e3][pick(4, 4)];
    random.max_keyframe_segments = [0, 1, 2, 32, 4096][pick(6, 5)];
    random.key_reduction_enabled = hash & (1 << 9) != 0;
    random.key_reduction_rotation = hash & (1 << 10) != 0;
    random.key_reduction_threshold = [-1.0, 0.0, 1e-6, 0.5, 1e9][pick(11, 5)];
    random.key_reduction_passes = [0, 1, 4, 64][pick(14, 4)];
    random.bake_transform_properties = hash & (1 << 16) != 0;
    random.max_keys = [0, 1, 17, 4096, 1 << 20][pick(17, 5)];
    random.max_allocation = [0, 4096, 1 << 20, 1 << 24][pick(20, 4)];
    random.max_evaluation_work = [0, 100, 10_000, 1 << 22][pick(22, 4)];
    sets.push(random);
    let nodes = scene.nodes().len();
    let stacks = scene.animation_stacks().len();
    let mut targets: Vec<Option<usize>> = vec![None];
    targets.extend((0..stacks.min(3)).map(Some));
    for options in &sets {
        for &stack in &targets {
            if let Ok(baked) = scene.bake_animation(stack, options) {
                let mut previous = None;
                for node in &baked.nodes {
                    assert!(node.node < nodes);
                    assert!(previous.is_none_or(|p| p < node.node));
                    previous = Some(node.node);
                }
                std::hint::black_box(&baked);
            }
        }
    }
    assert!(scene
        .bake_animation(Some(stacks), &BakeOptions::default())
        .is_err());
}

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
            times.extend(
                curve
                    .keys
                    .iter()
                    .take(8)
                    .flat_map(|k| [k.time, k.time + 1e-4]),
            );
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
        check_order(&scene);
        check_materials(&scene);
        check_bake(&scene, data);
    }
});
