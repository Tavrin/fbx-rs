use super::*;

fn check(valid: bool) -> R<()> {
    if valid {
        Ok(())
    } else {
        Err(ReadError::Corrupt)
    }
}
fn refs(values: &[usize], len: usize) -> R<()> {
    check(values.iter().all(|&v| v < len))
}
fn optional(value: Option<usize>, len: usize) -> R<()> {
    check(value.is_none_or(|v| v < len))
}
fn indices(values: &[u32], len: usize) -> R<()> {
    check(values.iter().all(|&v| (v as usize) < len))
}
fn range(begin: usize, count: usize, len: usize) -> R<()> {
    check(begin.checked_add(count).is_some_and(|end| end <= len))
}
fn attribute<const N: usize>(v: &VertexAttribute<N>, corners: usize) -> R<()> {
    check(v.values.len().is_multiple_of(N) && v.indices.len() == corners)?;
    indices(&v.indices, v.values.len() / N)
}

/// Validate the invariants relied on by public accessors, evaluation and baking.
///
/// * All node attachments, children, instances, materials, textures, bones,
///   poses and deformer links index the appropriate scene table.
/// * Node parents agree with evaluation parents and reciprocal children;
///   the evaluation order is a permutation with parents and scale dependencies
///   preceding consumers. This also proves the parent/scale graphs acyclic.
/// * Positions contain exactly the control points; attributes have whole
///   values and one valid index per corner. Faces partition the corners,
///   triangle corners are valid triples, and material parts reference faces
///   with consistent triangle counts and material slots.
/// * Skin influence ranges partition the weights and local cluster indices are valid; sparse skin,
///   dual-quaternion and blend arrays have matching lengths and control-point
///   indices valid for every attached mesh. Optional normal/weight arrays may
///   be empty. Blend offsets and targets are sorted; the final target agrees
///   with the last keyframe, as required by the loader.
/// * Animation element IDs, curves, values, bindings, layers, stacks,
///   property connections and helper nodes are in range. Binding and key
///   ordering supports partition searches; parallel evaluation tables match
///   the public nodes, channels and clusters. Mirror axes stay in 0..=3.
///
/// Work is linear in decoded tables and links, including shared deformers;
/// scratch ranks and propagated vertex bounds are budgeted. No
/// recursion or allocation based on an unchecked element count is used.
pub(super) fn validate(s: &Scene, budget: &Budget) -> R<()> {
    let a = &s.animation;
    let n = s.nodes.len();
    check(n > 0 && a.nodes.len() == n && a.node_order.len() == n)?;
    check(a.ids.len() == s.element_count && a.creation.len() == s.element_count)?;
    check(a.channels.len() == s.blend_channels.len() && a.clusters.len() == s.skin_clusters.len())?;
    let mut rank = budget.filled(n, usize::MAX).map_err(alloc_error)?;
    for (r, &i) in a.node_order.iter().enumerate() {
        let slot = rank.get_mut(i).ok_or(ReadError::Corrupt)?;
        check(*slot == usize::MAX)?;
        *slot = r;
    }
    check(s.nodes[0].parent.is_none())?;
    for (i, (node, eval)) in s.nodes.iter().zip(&a.nodes).enumerate() {
        check(eval.element < s.element_count && node.parent == eval.parent)?;
        check(i == 0 || node.parent.is_some_and(|p| p < i))?;
        check((0..=6).contains(&eval.rotation_order))?;
        check((0..=3).contains(&eval.adjust.mirror_axis))?;
        for dep in [eval.parent, eval.translation_scale, eval.inherited_scale]
            .into_iter()
            .flatten()
        {
            check(dep < n && rank[dep] < rank[i])?;
        }
        optional(eval.own_helper, n)?;
        optional(eval.inherit_helper, n)?;
        refs(&node.children, n)?;
        for &child in &node.children {
            check(s.nodes[child].parent == Some(i))?;
        }
        optional(node.mesh, s.meshes.len())?;
        optional(node.camera, s.cameras.len())?;
        optional(node.light, s.lights.len())?;
        optional(node.bone, s.bones.len())?;
        optional(node.bind_pose, s.poses.len())?;
        refs(&node.materials, s.materials.len())?;
    }
    // Count each child once and ensure every non-root parent has a child link.
    rank.fill(0);
    for node in &s.nodes {
        for &child in &node.children {
            check(rank[child] == 0)?;
            rank[child] = 1;
        }
    }
    for (i, node) in s.nodes.iter().enumerate() {
        check(rank[i] == usize::from(node.parent.is_some()))?;
    }
    for m in &s.meshes {
        let corners = m.positions.indices.len();
        check(m.positions.values.len() / 3 == m.num_vertices)?;
        attribute(&m.positions, corners)?;
        if let Some(v) = &m.normals {
            attribute(v, corners)?;
        }
        for uv in &m.uv_sets {
            attribute(&uv.uv, corners)?;
            if let Some(v) = &uv.tangent {
                attribute(v, corners)?;
            }
            if let Some(v) = &uv.bitangent {
                attribute(v, corners)?;
            }
        }
        for color in &m.color_sets {
            attribute(&color.color, corners)?;
        }
        let mut end = 0;
        for face in &m.faces {
            check(face.begin as usize == end)?;
            range(end, face.count as usize, corners)?;
            end += face.count as usize;
        }
        check(end == corners && m.triangles.len().is_multiple_of(3))?;
        indices(&m.triangles, corners)?;
        refs(&m.materials, s.materials.len())?;
        check(m.face_material.is_empty() || m.face_material.len() == m.faces.len())?;
        indices(&m.face_material, m.materials.len().max(1))?;
        check(m.parts.len() == m.materials.len().max(1))?;
        for (i, part) in m.parts.iter().enumerate() {
            check(part.index as usize == i)?;
            indices(&part.face_indices, m.faces.len())?;
            let triangles: u64 = part
                .face_indices
                .iter()
                .map(|&f| u64::from(m.faces[f as usize].count.saturating_sub(2)))
                .sum();
            check(triangles == part.num_triangles)?;
        }
        refs(&m.instances, n)?;
        refs(&m.skin_deformers, s.skin_deformers.len())?;
        refs(&m.blend_deformers, s.blend_deformers.len())?;
    }
    for material in &s.materials {
        for (_, map) in material.fbx.iter().chain(&material.pbr) {
            optional(map.texture, s.textures.len())?;
        }
        for t in &material.textures {
            check(t.texture < s.textures.len())?;
        }
    }
    for t in &s.textures {
        refs(&t.file_textures, s.textures.len())?;
        for layer in &t.layers {
            check(layer.texture < s.textures.len())?;
        }
    }
    for b in &s.bones {
        refs(&b.instances, n)?;
    }
    for skin in &s.skin_deformers {
        refs(&skin.clusters, s.skin_clusters.len())?;
        let mut end = 0;
        for v in &skin.vertices {
            check(v.weight_begin == end)?;
            range(v.weight_begin, v.num_weights, skin.weights.len())?;
            end += v.num_weights;
        }
        check(end == skin.weights.len())?;
        check(
            skin.max_weights_per_vertex
                == skin
                    .vertices
                    .iter()
                    .map(|v| v.num_weights)
                    .max()
                    .unwrap_or(0),
        )?;
        for w in &skin.weights {
            check(w.cluster_index < skin.clusters.len())?;
        }
        check(skin.dq_vertices.len() == skin.dq_weights.len())?;
        indices(&skin.dq_vertices, skin.vertices.len())?;
    }
    for c in &s.skin_clusters {
        optional(c.bone_node, n)?;
        check(c.vertices.len() == c.weights.len())?;
    }
    for b in &s.blend_deformers {
        refs(&b.channels, s.blend_channels.len())?;
    }
    for c in &s.blend_channels {
        optional(c.target_shape, s.blend_shapes.len())?;
        check(c.target_shape == c.keyframes.last().map(|k| k.shape))?;
        check(c.keyframes.windows(2).all(|w| {
            w[0].target_weight.partial_cmp(&w[1].target_weight) != Some(std::cmp::Ordering::Greater)
        }))?;
        for k in &c.keyframes {
            check(k.shape < s.blend_shapes.len())?;
        }
    }
    for shape in &s.blend_shapes {
        let len = shape.offset_vertices.len();
        check(
            shape.position_offsets.len() == len
                && (shape.normal_offsets.is_empty() || shape.normal_offsets.len() == len)
                && (shape.offset_weights.is_empty() || shape.offset_weights.len() == len),
        )?;
        check(shape.offset_vertices.windows(2).all(|w| w[0] <= w[1]))?;
    }
    // Propagate the tightest mesh bound once through each deformer table.
    // Repeated or shared links must not multiply payload scans.
    let mut skin_bounds = budget
        .filled(s.skin_deformers.len(), usize::MAX)
        .map_err(alloc_error)?;
    let mut cluster_bounds = budget
        .filled(s.skin_clusters.len(), usize::MAX)
        .map_err(alloc_error)?;
    let mut blend_bounds = budget
        .filled(s.blend_deformers.len(), usize::MAX)
        .map_err(alloc_error)?;
    let mut channel_bounds = budget
        .filled(s.blend_channels.len(), usize::MAX)
        .map_err(alloc_error)?;
    let mut shape_bounds = budget
        .filled(s.blend_shapes.len(), usize::MAX)
        .map_err(alloc_error)?;
    for m in &s.meshes {
        for &i in &m.skin_deformers {
            check(s.skin_deformers[i].vertices.len() >= m.num_vertices)?;
            skin_bounds[i] = skin_bounds[i].min(m.num_vertices);
        }
        for &i in &m.blend_deformers {
            blend_bounds[i] = blend_bounds[i].min(m.num_vertices);
        }
    }
    for (skin, &bound) in s.skin_deformers.iter().zip(&skin_bounds) {
        for &c in &skin.clusters {
            cluster_bounds[c] = cluster_bounds[c].min(bound);
        }
    }
    for (cluster, &bound) in s.skin_clusters.iter().zip(&cluster_bounds) {
        indices(&cluster.vertices, bound)?;
    }
    for (blend, &bound) in s.blend_deformers.iter().zip(&blend_bounds) {
        for &c in &blend.channels {
            channel_bounds[c] = channel_bounds[c].min(bound);
        }
    }
    for (channel, &bound) in s.blend_channels.iter().zip(&channel_bounds) {
        for shape in channel
            .keyframes
            .iter()
            .map(|k| k.shape)
            .chain(channel.target_shape)
        {
            shape_bounds[shape] = shape_bounds[shape].min(bound);
        }
    }
    for (shape, &bound) in s.blend_shapes.iter().zip(&shape_bounds) {
        indices(&shape.offset_vertices, bound)?;
    }
    for pose in &s.poses {
        for b in &pose.bone_poses {
            check(b.bone_node < n)?;
        }
    }
    optional(a.default_stack, a.stacks.len())?;
    for curve in &a.curves {
        check(curve.element < s.element_count)?;
        check(
            curve
                .keys
                .windows(2)
                .all(|w| w[0].ticks <= w[1].ticks && w[0].time <= w[1].time),
        )?;
    }
    for value in &a.values {
        check(value.element < s.element_count)?;
        for &curve in &value.curves {
            optional(curve, a.curves.len())?;
        }
    }
    for layer in &a.layers {
        check(layer.element < s.element_count)?;
        refs(&layer.values, a.values.len())?;
        for b in &layer.bindings {
            check(b.element < s.element_count && b.value < a.values.len())?;
        }
        check(
            layer
                .bindings
                .windows(2)
                .all(|w| (w[0].element, &w[0].property) <= (w[1].element, &w[1].property)),
        )?;
    }
    for stack in &a.stacks {
        check(stack.element < s.element_count)?;
        refs(&stack.layers, a.layers.len())?;
    }
    for ((element, _), _) in &a.properties {
        check(*element < s.element_count)?;
    }
    for ((element, _), (target, _)) in &a.connections {
        check(*element < s.element_count && *target < s.element_count)?;
    }
    refs(&a.creation, s.element_count)?;
    refs(&a.channels, s.element_count)?;
    for ((bone, _), cluster) in a.clusters.iter().zip(&s.skin_clusters) {
        optional(*bone, n)?;
        check(*bone == cluster.bone_node)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Scene {
        crate::load(
            include_bytes!("../../tests/fixtures/allocation-rich.fbx"),
            &LoadOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn validation_scratch_respects_allocation_limit() {
        let scene = fixture();
        assert_eq!(
            validate(&scene, &Budget::new(0)),
            Err(ReadError::Limit(Limit::Allocation))
        );
    }

    /// Derive field patches with the format's encoder, apply them to a valid
    /// sidecar, and recompute its trailer. This includes variable-size fields
    /// and the corresponding section offsets, without hardcoded byte offsets.
    fn reject(change: impl FnOnce(&mut Scene)) {
        let mut scene = fixture();
        let key = CacheKey::new(
            b"structural-test",
            &LoadOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        scene.write_cache(&key, &mut bytes).unwrap();
        assert!(read(&bytes, &key, &Limits::default()).is_ok());
        change(&mut scene);
        let mut patched = Vec::new();
        scene.write_cache(&key, &mut patched).unwrap();
        let end = patched.len() - 8;
        bytes.resize(patched.len(), 0);
        for (dst, &src) in bytes[..end].iter_mut().zip(&patched[..end]) {
            if *dst != src {
                *dst = src;
            }
        }
        bytes[end..end + 4]
            .copy_from_slice(&miniz_oxide::mz_adler32_oxide(1, &patched[..end]).to_le_bytes());
        bytes[end + 4..].copy_from_slice(&TRAILER_TAG);
        assert_eq!(
            read(&bytes, &key, &Limits::default()).err(),
            Some(ReadError::Corrupt)
        );
    }

    #[test]
    #[ignore = "paired release timing; optional external fixture"]
    fn read_validation_cost() {
        let path = std::env::var("FBX_CACHE_BENCH_INPUT")
            .unwrap_or_else(|_| "tests/fixtures/pbr-materials.fbx".into());
        let input = std::fs::read(&path).unwrap();
        let limits = Limits::default();
        let scene = crate::load_with_limits(&input, &LoadOptions::default(), &limits).unwrap();
        let key = CacheKey::new(b"timing", &LoadOptions::default(), &limits).unwrap();
        let mut bytes = Vec::new();
        scene.write_cache(&key, &mut bytes).unwrap();
        drop(scene);
        let mut times = [Vec::new(), Vec::new()];
        for pair in 0..7 {
            for mode in if pair % 2 == 0 { [0, 1] } else { [1, 0] } {
                let start = std::time::Instant::now();
                let decoded = if mode == 0 {
                    read_validated(std::hint::black_box(&bytes), &key, &limits, |_, _| Ok(()))
                } else {
                    read(std::hint::black_box(&bytes), &key, &limits)
                }
                .unwrap();
                times[mode].push(start.elapsed());
                std::hint::black_box(&decoded);
                drop(decoded);
            }
        }
        for t in &mut times {
            t.sort();
        }
        eprintln!("input {} bytes, cache {} bytes; 7 alternating pairs: without {:?}, with {:?}, overhead {:.2}%",
            input.len(), bytes.len(), times[0][3], times[1][3],
            (times[1][3].as_secs_f64() / times[0][3].as_secs_f64() - 1.0) * 100.0);
    }

    macro_rules! cases {
        ($($name:ident: $change:expr),+ $(,)?) => { $(#[test] fn $name() { reject($change); })+ };
    }
    cases! {
        empty_hierarchy: |s| { s.nodes.clear(); s.animation.nodes.clear(); s.animation.node_order.clear(); },
        node_mesh: |s| s.nodes[1].mesh = Some(s.meshes.len()),
        node_material: |s| s.nodes[1].materials.push(s.materials.len()),
        node_camera: |s| s.nodes[1].camera = Some(s.cameras.len()),
        node_light: |s| s.nodes[1].light = Some(s.lights.len()),
        node_bone: |s| s.nodes[1].bone = Some(s.bones.len()),
        node_pose: |s| s.nodes[1].bind_pose = Some(s.poses.len()),
        node_parent: |s| { s.nodes[1].parent = Some(s.nodes.len()); s.animation.nodes[1].parent = s.nodes[1].parent; },
        parent_cycle: |s| { s.nodes[0].parent = Some(1); s.animation.nodes[0].parent = Some(1); },
        child_index: |s| { let n = s.nodes.len(); s.nodes[0].children.push(n); },
        child_parent: |s| s.nodes[1].children.push(0),
        missing_child: |s| { s.nodes[0].children.pop(); },
        duplicate_child: |s| { let child = s.nodes[0].children[0]; s.nodes[0].children.push(child); },
        node_orphan: |s| { s.nodes[1].parent = None; s.animation.nodes[1].parent = None; },
        rotation_order: |s| s.animation.nodes[1].rotation_order = 7,
        eval_parent: |s| s.animation.nodes[1].parent = None,
        eval_count: |s| { s.animation.nodes.pop(); },
        eval_element: |s| s.animation.nodes[1].element = s.element_count,
        eval_order_index: |s| s.animation.node_order[0] = s.nodes.len(),
        eval_order_duplicate: |s| s.animation.node_order[1] = s.animation.node_order[0],
        eval_order_missing: |s| { s.animation.node_order.pop(); },
        eval_order_dependency: |s| s.animation.node_order.reverse(),
        translation_dependency: |s| s.animation.nodes[1].translation_scale = Some(s.nodes.len()),
        translation_cycle: |s| s.animation.nodes[1].translation_scale = Some(1),
        scale_dependency: |s| s.animation.nodes[1].inherited_scale = Some(s.nodes.len()),
        scale_cycle: |s| s.animation.nodes[1].inherited_scale = Some(1),
        own_helper: |s| s.animation.nodes[1].own_helper = Some(s.nodes.len()),
        inherit_helper: |s| s.animation.nodes[1].inherit_helper = Some(s.nodes.len()),
        mirror_negative: |s| s.animation.nodes[1].adjust.mirror_axis = -1,
        mirror_large: |s| s.animation.nodes[1].adjust.mirror_axis = 4,
        position_count: |s| s.meshes[0].num_vertices += 1,
        position_components: |s| s.meshes[0].positions.values.push(0.0),
        position_index: |s| s.meshes[0].positions.indices[0] = u32::MAX,
        normal_components: |s| s.meshes[0].normals.as_mut().unwrap().values.push(0.0),
        uv_corners: |s| { s.meshes[0].uv_sets[0].uv.indices.pop(); },
        color_components: |s| s.meshes[0].color_sets[0].color.values.push(0.0),
        normal_corners: |s| { s.meshes[0].normals.as_mut().unwrap().indices.pop(); },
        normal_index: |s| s.meshes[0].normals.as_mut().unwrap().indices[0] = u32::MAX,
        uv_index: |s| s.meshes[0].uv_sets[0].uv.indices[0] = u32::MAX,
        tangent_index: |s| { let mut v = s.meshes[0].normals.clone().unwrap(); v.indices[0] = u32::MAX; s.meshes[0].uv_sets[0].tangent = Some(v); },
        bitangent_index: |s| { let mut v = s.meshes[0].normals.clone().unwrap(); v.indices[0] = u32::MAX; s.meshes[0].uv_sets[0].bitangent = Some(v); },
        color_index: |s| s.meshes[0].color_sets[0].color.indices[0] = u32::MAX,
        face_begin: |s| s.meshes[0].faces[0].begin = u32::MAX,
        face_count: |s| s.meshes[0].faces[0].count = u32::MAX,
        face_partition: |s| s.meshes[0].faces[0].count = 0,
        triangle_index: |s| s.meshes[0].triangles[0] = u32::MAX,
        triangle_triples: |s| { s.meshes[0].triangles.pop(); },
        mesh_material: |s| s.meshes[0].materials.push(s.materials.len()),
        face_material: |s| s.meshes[0].face_material[0] = u32::MAX,
        face_material_count: |s| s.meshes[0].face_material.push(0),
        part_index: |s| s.meshes[0].parts[0].index = u32::MAX,
        part_face: |s| s.meshes[0].parts[0].face_indices[0] = u32::MAX,
        part_triangles: |s| s.meshes[0].parts[0].num_triangles += 1,
        part_count: |s| { s.meshes[0].parts.pop(); },
        mesh_instance: |s| s.meshes[0].instances.push(s.nodes.len()),
        mesh_skin: |s| s.meshes[0].skin_deformers.push(s.skin_deformers.len()),
        mesh_blend: |s| s.meshes[0].blend_deformers.push(s.blend_deformers.len()),
        material_map: |s| s.materials[0].fbx[0].1.texture = Some(s.textures.len()),
        pbr_map: |s| s.materials[0].pbr[0].1.texture = Some(s.textures.len()),
        material_texture: |s| s.materials[0].textures[0].texture = s.textures.len(),
        texture_layer: |s| { let t = s.textures.iter_mut().find(|t| !t.layers.is_empty()).unwrap(); t.layers[0].texture = usize::MAX; },
        file_texture: |s| { let n = s.textures.len(); s.textures[0].file_textures.push(n); },
        bone_instance: |s| s.bones[0].instances.push(s.nodes.len()),
        skin_cluster: |s| s.skin_deformers[0].clusters[0] = s.skin_clusters.len(),
        weight_begin: |s| s.skin_deformers[0].vertices[0].weight_begin = usize::MAX,
        weight_partition: |s| s.skin_deformers[0].vertices[1].weight_begin = 0,
        weight_count: |s| s.skin_deformers[0].vertices[0].num_weights = usize::MAX,
        weight_cluster: |s| s.skin_deformers[0].weights[0].cluster_index = usize::MAX,
        max_weights: |s| s.skin_deformers[0].max_weights_per_vertex += 1,
        dq_count: |s| s.skin_deformers[0].dq_weights.push(1.0),
        dq_vertex: |s| { s.skin_deformers[0].dq_vertices.push(u32::MAX); s.skin_deformers[0].dq_weights.push(1.0); },
        cluster_bone: |s| s.skin_clusters[0].bone_node = Some(s.nodes.len()),
        cluster_weights: |s| s.skin_clusters[0].weights.push(1.0),
        cluster_vertex: |s| s.skin_clusters[0].vertices[0] = u32::MAX,
        skin_vertex_count: |s| { s.skin_deformers[0].vertices.pop(); },
        blend_channel: |s| s.blend_deformers[0].channels[0] = s.blend_channels.len(),
        blend_target: |s| s.blend_channels[0].target_shape = Some(s.blend_shapes.len()),
        blend_last_target: |s| s.blend_channels[0].target_shape = None,
        blend_key_order: |s| { let c = &mut s.blend_channels[0]; let mut k = c.keyframes[0].clone(); k.target_weight = c.keyframes[0].target_weight - 1.0; c.keyframes.push(k); },
        blend_key_shape: |s| s.blend_channels[0].keyframes[0].shape = s.blend_shapes.len(),
        shape_position: |s| { s.blend_shapes[0].position_offsets.pop(); },
        shape_normal: |s| s.blend_shapes[0].normal_offsets = vec![[0.0; 3]; 2],
        shape_weight: |s| s.blend_shapes[0].offset_weights = vec![1.0; 2],
        shape_vertex: |s| s.blend_shapes[0].offset_vertices[0] = u32::MAX,
        shape_order: |s| { let v = &mut s.blend_shapes[0]; v.offset_vertices = vec![1, 0]; v.position_offsets = vec![[0.0; 3]; 2]; },
        pose_bone: |s| s.poses[0].bone_poses[0].bone_node = s.nodes.len(),
        default_stack: |s| s.animation.default_stack = Some(s.animation.stacks.len()),
        curve_element: |s| s.animation.curves[0].element = s.element_count,
        curve_order: |s| s.animation.curves[0].keys.reverse(),
        value_element: |s| s.animation.values[0].element = s.element_count,
        value_curve: |s| s.animation.values[0].curves[0] = Some(s.animation.curves.len()),
        layer_element: |s| s.animation.layers[0].element = s.element_count,
        layer_value: |s| s.animation.layers[0].values.push(s.animation.values.len()),
        binding_element: |s| s.animation.layers[0].bindings[0].element = s.element_count,
        binding_value: |s| s.animation.layers[0].bindings[0].value = s.animation.values.len(),
        binding_order: |s| { let layer = &mut s.animation.layers[0]; let mut b = layer.bindings[0].clone(); b.element = 0; layer.bindings.push(b); },
        stack_element: |s| s.animation.stacks[0].element = s.element_count,
        stack_layer: |s| s.animation.stacks[0].layers[0] = s.animation.layers.len(),
        property_element: |s| { s.animation.properties.try_insert((s.element_count, Vec::new()), EvaluatedProperty::default(), &Budget::new(usize::MAX)).unwrap(); },
        connection_element: |s| { s.animation.connections.try_insert((s.element_count, Vec::new()), (0, Vec::new()), &Budget::new(usize::MAX)).unwrap(); },
        connection_target: |s| { s.animation.connections.try_insert((0, Vec::new()), (s.element_count, Vec::new()), &Budget::new(usize::MAX)).unwrap(); },
        animation_ids: |s| { s.animation.ids.pop(); },
        animation_creation_index: |s| s.animation.creation[0] = s.element_count,
        animation_creation: |s| { s.animation.creation.pop(); },
        animation_channel: |s| s.animation.channels[0] = s.element_count,
        animation_channel_count: |s| { s.animation.channels.pop(); },
        animation_cluster: |s| s.animation.clusters[0].0 = Some(s.nodes.len()),
        animation_cluster_binding: |s| s.animation.clusters[0].0 = Some(0),
        animation_cluster_count: |s| { s.animation.clusters.pop(); }
    }
}
