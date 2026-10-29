//! Static deformer conversion following pinned ufbx 0.23.1:
//! read_skin(_cluster), read_shape, finalize_scene, update_initial_clusters,
//! update_pose, update_skin_cluster and update_blend_channel.
use crate::canon::{Obj, V};
use crate::canon_dom::{node_data, NodeData, Scalar};
use crate::canon_scene::{CanonScene, DumpLimits, ElementType as T};
use crate::deform::*;
use crate::scene_node as sn;
use crate::{Matrix, Transform};

#[derive(Default)]
pub(crate) struct Deformers {
    pub skins: Vec<(usize, SkinDeformer)>,
    pub clusters: Vec<(usize, SkinCluster)>,
    pub blends: Vec<(usize, BlendDeformer)>,
    pub channels: Vec<(usize, BlendChannel)>,
    pub shapes: Vec<(usize, BlendShape)>,
    pub poses: Vec<(usize, Pose)>,
}
struct Reader<'a, 'd> {
    s: &'a CanonScene<'d>,
    children: Vec<Vec<usize>>,
    used: std::cell::Cell<usize>,
    limit: usize,
}
impl Reader<'_, '_> {
    fn charge(&self, count: usize, width: usize) -> Result<(), String> {
        let used = count
            .checked_mul(width)
            .and_then(|n| self.used.get().checked_add(n))
            .ok_or("deformer: allocation limit")?;
        if used > self.limit {
            return Err("deformer: allocation limit".into());
        }
        self.used.set(used);
        Ok(())
    }
    fn child(&self, n: usize, name: &[u8]) -> Option<usize> {
        self.children[n]
            .iter()
            .copied()
            .find(|&c| self.s.doc.nodes[c].name == name)
    }
    fn data(&self, n: usize, name: &[u8]) -> Option<NodeData> {
        self.child(n, name)
            .map(|c| node_data(&self.s.doc, c, self.s.ascii))
    }
    fn reals(&self, n: usize, name: &[u8]) -> Result<Vec<f64>, String> {
        match self.data(n, name) {
            None => Ok(Vec::new()),
            Some(NodeData::Reals(v)) if v.iter().all(|x| x.is_finite()) => {
                self.charge(v.len(), 32)?;
                Ok(v)
            }
            _ => Err(format!("deformer: invalid real array at {n}")),
        }
    }
    fn indices(&self, n: usize, name: &[u8]) -> Result<Vec<u32>, String> {
        match self.data(n, name) {
            None => Ok(Vec::new()),
            Some(NodeData::Ints(v)) if v.iter().all(|&x| x >= 0) => {
                self.charge(v.len(), 16)?;
                Ok(v.into_iter().map(|x| x as u32).collect())
            }
            _ => Err(format!("deformer: invalid index array at {n}")),
        }
    }
    fn matrix(&self, n: usize, name: &[u8]) -> Result<sn::Mat, String> {
        let v = self.reals(n, name)?;
        if v.is_empty() && self.child(n, name).is_none() {
            return Ok([0.0; 12]);
        }
        if v.len() != 16 {
            return Err("deformer: matrix must have 16 values".into());
        }
        Ok(std::array::from_fn(|i| v[i / 3 * 4 + i % 3]))
    }
    fn dst(&self, i: usize, ty: T) -> Vec<usize> {
        let mut out = Vec::new();
        for (k, p) in &self.s.elements[i].dst_conns {
            if p.is_empty() && self.s.elements[*k].ty == ty && !out.contains(k) {
                out.push(*k);
            }
        }
        out
    }
    fn src(&self, i: usize, ty: T) -> Vec<usize> {
        let mut out = Vec::new();
        for (k, p) in &self.s.elements[i].src_conns {
            if p.is_empty() && self.s.elements[*k].ty == ty && !out.contains(k) {
                out.push(*k);
            }
        }
        out
    }
    fn mesh_targets(&self, i: usize) -> Vec<usize> {
        let mut out = self.src(i, T::Mesh);
        for node in self.src(i, T::Node) {
            out.extend(self.dst(node, T::Mesh));
        }
        out
    }
    fn num_vertices(&self, mesh: usize) -> Result<usize, String> {
        Ok(self
            .reals(
                self.s.elements[mesh]
                    .node
                    .ok_or("deformer: mesh has no source")?,
                b"Vertices",
            )?
            .len()
            / 3)
    }
    fn source_id(&self, i: usize) -> Option<i64> {
        let e = &self.s.elements[i];
        if e.node.is_some_and(|n| self.s.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        }
    }
    fn name(&self, i: usize) -> String {
        String::from_utf8_lossy(&self.s.elements[i].name).into_owned()
    }
}
fn refs(v: &[usize]) -> V {
    V::List(v.iter().map(|&k| V::Ref(Some(k))).collect())
}
fn ints(v: &[u32]) -> V {
    V::List(v.iter().map(|&k| V::U(k.into())).collect())
}
fn vecs(v: &[[f64; 3]]) -> V {
    V::List(v.iter().map(|v| V::Vec(v.to_vec())).collect())
}
fn tr(t: &sn::Transform) -> Transform {
    Transform {
        translation: t.translation,
        rotation: t.rotation,
        scale: t.scale,
    }
}
fn tr_obj(t: &Transform) -> V {
    V::Obj(
        Obj::new()
            .with("translation", V::Vec(t.translation.to_vec()))
            .with("rotation", V::Vec(t.rotation.to_vec()))
            .with("scale", V::Vec(t.scale.to_vec())),
    )
}
fn mirror(m: &mut sn::Mat, axis: i64) {
    if axis == 0 {
        return;
    }
    let a = axis as usize - 1;
    for r in 0..3 {
        m[a * 3 + r] = -m[a * 3 + r];
    }
    sn::mirror_matrix_dst(m, axis);
}
fn metadata_real(s: &CanonScene<'_>, name: &str, def: f64) -> f64 {
    match s.metadata.get(name) {
        Some(V::F(x)) => *x,
        _ => def,
    }
}

/// Convert, validate and retain typed data for both the API and canonical dump.
pub(crate) fn project(s: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    if !s.elements.iter().any(|e| {
        matches!(
            e.ty,
            T::SkinDeformer
                | T::SkinCluster
                | T::BlendDeformer
                | T::BlendChannel
                | T::BlendShape
                | T::Pose
        )
    }) {
        return Ok(());
    }
    let mut children = vec![Vec::new(); s.doc.nodes.len()];
    for (i, n) in s.doc.nodes.iter().enumerate() {
        if let Some(p) = n.parent {
            children[p].push(i);
        }
    }
    let r = Reader {
        s,
        children,
        used: std::cell::Cell::new(0),
        limit: limits.max_allocation,
    };
    r.charge(s.doc.nodes.len(), std::mem::size_of::<Vec<usize>>() + 8)?;
    r.charge(s.elements.len(), 64)?;
    // Reject deformer-only cycles, including unconnected cycles, without recursion.
    let mut color = vec![0u8; s.elements.len()];
    for start in 0..s.elements.len() {
        if !matches!(
            s.elements[start].ty,
            T::SkinDeformer | T::SkinCluster | T::BlendDeformer | T::BlendChannel | T::BlendShape
        ) || color[start] != 0
        {
            continue;
        }
        let mut stack = vec![(start, 0)];
        color[start] = 1;
        while let Some((i, next)) = stack.last_mut() {
            let conns = &s.elements[*i].src_conns;
            if *next == conns.len() {
                color[*i] = 2;
                stack.pop();
                continue;
            }
            let (k, p) = &conns[*next];
            *next += 1;
            if !p.is_empty()
                || !matches!(
                    s.elements[*k].ty,
                    T::SkinDeformer
                        | T::SkinCluster
                        | T::BlendDeformer
                        | T::BlendChannel
                        | T::BlendShape
                )
            {
                continue;
            }
            if color[*k] == 1 {
                return Err("deformer: connection cycle".into());
            }
            if color[*k] == 0 {
                color[*k] = 1;
                stack.push((*k, 0));
            }
        }
    }
    let scale = metadata_real(s, "geometry_scale", 1.0);
    let axis = match s.metadata.get("mirror_axis") {
        Some(V::I(x)) => *x,
        _ => 0,
    };
    let root_scale = metadata_real(s, "root_scale", 1.0);
    let root_rot = match s.metadata.get("root_rotation") {
        Some(V::Vec(v)) if v.len() == 4 => [v[0], v[1], v[2], v[3]],
        _ => [0.0, 0.0, 0.0, 1.0],
    };
    let world_to_units = sn::transform_to_matrix(&sn::Transform {
        translation: [0.0; 3],
        rotation: root_rot,
        scale: [root_scale; 3],
    });
    let convert_bind = |mut m: sn::Mat| {
        m = sn::matrix_mul(&world_to_units, &m);
        for v in &mut m[9..12] {
            *v *= scale;
        }
        mirror(&mut m, axis);
        m
    };
    let mut d = Deformers::default();
    let mut node_bind = vec![None; s.elements.len()];
    // Resolve poses in source order: the first bind pose wins for each node.
    let parent_of =
        |k: usize| {
            s.typed.parent.get(k).copied().flatten().or_else(|| {
                match s.elements[k].obj.get("parent") {
                    Some(V::Ref(p)) => *p,
                    _ => None,
                }
            })
        };
    let mut pose_ids: Vec<usize> = s
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| e.ty == T::Pose)
        .map(|(i, _)| i)
        .collect();
    pose_ids.sort_by_key(|&i| s.elements[i].ordinal);
    for i in pose_ids {
        let e = &s.elements[i];
        let n = e.node.ok_or("pose: missing source")?;
        let props = &s.doc.properties[s.doc.nodes[n].properties.clone()];
        let bind = matches!(props.get(2),Some(crate::syntax::Property::String(v)) if v.as_ref()==b"BindPose");
        let mut bones = Vec::new();
        for &pn in &r.children[n] {
            if s.doc.nodes[pn].name != b"PoseNode" {
                continue;
            }
            let id = match r.data(pn, b"Node") {
                Some(NodeData::Values(v)) => match v.first() {
                    Some(Scalar::Num(x, _)) => *x,
                    _ => return Err("pose: invalid node".into()),
                },
                _ => return Err("pose: missing node".into()),
            };
            let k = s
                .elements
                .iter()
                .position(|e| e.source_id == Some(id) && e.ty == T::Node)
                .ok_or("pose: invalid node ID")?;
            if r.child(pn, b"Matrix").is_none() {
                return Err("pose: missing matrix".into());
            }
            let m = convert_bind(r.matrix(pn, b"Matrix")?);
            if bind && node_bind[k].is_none() {
                node_bind[k] = Some(i);
            }
            bones.push(BonePose {
                bone_node: k,
                bone_to_world: Matrix(m),
                bone_to_parent: Matrix([0.0; 12]),
            });
        }
        // ufbx typed node order: depth, original parent element ID, original ID.
        bones.sort_by_key(|b| {
            let mut depth = 0usize;
            let mut at = parent_of(b.bone_node);
            while let Some(k) = at {
                depth += 1;
                at = parent_of(k);
                if depth > s.elements.len() {
                    break;
                }
            }
            (
                depth,
                parent_of(b.bone_node).map(|k| s.elements[k].ordinal),
                s.elements[b.bone_node].ordinal,
            )
        });
        for j in 0..bones.len() {
            let parent = s
                .typed
                .parent
                .get(bones[j].bone_node)
                .copied()
                .flatten()
                .or_else(|| match s.elements[bones[j].bone_node].obj.get("parent") {
                    Some(V::Ref(p)) => *p,
                    _ => None,
                });
            let pm = parent
                .and_then(|p| {
                    bones
                        .iter()
                        .find(|b| b.bone_node == p)
                        .map(|b| b.bone_to_world.0)
                        .or_else(|| s.node_states[p].as_ref().map(|st| st.node_to_world))
                })
                .unwrap_or(sn::IDENTITY_MATRIX);
            bones[j].bone_to_parent = Matrix(sn::matrix_mul(
                &sn::matrix_invert(&pm),
                &bones[j].bone_to_world.0,
            ));
        }
        d.poses.push((
            i,
            Pose {
                name: r.name(i),
                source_id: r.source_id(i),
                is_bind_pose: bind,
                bone_poses: bones,
            },
        ));
    }
    for (i, e) in s.elements.iter().enumerate() {
        let Some(n) = e.node else {
            continue;
        };
        match e.ty {
            T::BlendShape => {
                if r.child(n, b"Indexes").is_none() || r.child(n, b"Vertices").is_none() {
                    return Err("blend shape: missing offsets".into());
                }
                let indices = r.indices(n, b"Indexes")?;
                let pos = r.reals(n, b"Vertices")?;
                let normal = r.reals(n, b"Normals")?;
                if indices.len().checked_mul(3) != Some(pos.len())
                    || (!normal.is_empty() && normal.len() != pos.len())
                {
                    return Err("blend shape: offset count mismatch".into());
                }
                // Validate against every connected mesh, not merely the largest.
                for channel in r.src(i, T::BlendChannel) {
                    for blend in r.src(channel, T::BlendDeformer) {
                        for mesh in r.mesh_targets(blend) {
                            let count = r.num_vertices(mesh)?;
                            if indices.iter().any(|&v| v as usize >= count) {
                                return Err("blend shape: vertex out of range".into());
                            }
                        }
                    }
                }
                let mut order: Vec<usize> = (0..indices.len()).collect();
                order.sort_by_key(|&j| indices[j]);
                let offsets = |values: &[f64], scale: f64| -> Vec<[f64; 3]> {
                    if values.is_empty() {
                        return Vec::new();
                    }
                    order
                        .iter()
                        .map(|&j| {
                            let mut v = [values[j * 3], values[j * 3 + 1], values[j * 3 + 2]];
                            if scale != 1.0 {
                                for x in &mut v {
                                    *x *= scale;
                                }
                            }
                            if axis != 0 {
                                v[axis as usize - 1] = -v[axis as usize - 1];
                            }
                            v
                        })
                        .collect()
                };
                d.shapes.push((
                    i,
                    BlendShape {
                        name: r.name(i),
                        source_id: r.source_id(i),
                        offset_vertices: order.iter().map(|&j| indices[j]).collect(),
                        position_offsets: offsets(&pos, scale),
                        normal_offsets: offsets(&normal, 1.0),
                        offset_weights: Vec::new(),
                    },
                ));
            }
            T::SkinCluster => {
                let vertices = r.indices(n, b"Indexes")?;
                let weights = r.reals(n, b"Weights")?;
                if vertices.len() != weights.len() {
                    return Err("skin cluster: weight count mismatch".into());
                }
                let bone = r.dst(i, T::Node).first().copied();
                let mut mesh_mat = r.matrix(n, b"Transform")?;
                let mut bind = r.matrix(n, b"TransformLink")?;
                if bind.iter().all(|&x| x == 0.0) {
                    if let Some(k) = bone {
                        for (_, pose) in &d.poses {
                            if pose.is_bind_pose {
                                if let Some(b) = pose.bone_poses.iter().find(|b| b.bone_node == k) {
                                    bind = b.bone_to_world.0;
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    bind = convert_bind(bind);
                }
                let mut geom = mesh_mat;
                if let Some(skin) = r.src(i, T::SkinDeformer).first().copied() {
                    let meshes = r.mesh_targets(skin);
                    for &mesh in &meshes {
                        let count = r.num_vertices(mesh)?;
                        if vertices.iter().any(|&v| v as usize >= count) {
                            return Err("skin cluster: vertex out of range".into());
                        }
                    }
                    let node = r.src(skin, T::Node).first().copied().or_else(|| {
                        meshes
                            .first()
                            .and_then(|&m| s.elements[m].instances.first().copied())
                    });
                    if let Some(node) = node {
                        if mesh_mat.iter().all(|&x| x == 0.0) {
                            mesh_mat = sn::matrix_mul(
                                &sn::matrix_invert(&bind),
                                &s.node_states[node]
                                    .as_ref()
                                    .ok_or("skin: missing node state")?
                                    .node_to_world,
                            );
                        } else {
                            mirror(&mut mesh_mat, axis);
                            if scale != 1.0 {
                                for x in &mut mesh_mat[9..12] {
                                    *x *= scale;
                                }
                            }
                        }
                        // Geometry is baked by MODIFY_GEOMETRY, so the final geometry transform is identity.
                        geom = mesh_mat;
                    }
                }
                let bone_world = bone
                    .and_then(|k| s.node_states[k].as_ref().map(|n| n.node_to_world))
                    .unwrap_or(bind);
                let world = sn::matrix_mul(&bone_world, &geom);
                d.clusters.push((
                    i,
                    SkinCluster {
                        name: r.name(i),
                        source_id: r.source_id(i),
                        bone_node: bone,
                        geometry_to_bone: Matrix(geom),
                        mesh_node_to_bone: Matrix(mesh_mat),
                        bind_to_world: Matrix(bind),
                        geometry_to_world: Matrix(world),
                        geometry_to_world_transform: tr(&sn::matrix_to_transform(&world)),
                        vertices,
                        weights,
                    },
                ));
            }
            _ => {}
        }
    }
    for (i, e) in s.elements.iter().enumerate() {
        let Some(n) = e.node else {
            continue;
        };
        match e.ty {
            T::SkinDeformer => {
                let method = match r.data(n, b"SkinningType") {
                    Some(NodeData::Values(v)) => match v.first() {
                        Some(Scalar::Str(v)) => match v.as_slice() {
                            b"Rigid" => SkinningMethod::Rigid,
                            b"DualQuaternion" => SkinningMethod::DualQuaternion,
                            b"Blend" => SkinningMethod::BlendedDqLinear,
                            _ => SkinningMethod::Linear,
                        },
                        _ => SkinningMethod::Linear,
                    },
                    _ => SkinningMethod::Linear,
                };
                let clusters: Vec<usize> = r
                    .dst(i, T::SkinCluster)
                    .into_iter()
                    .filter(|k| {
                        d.clusters
                            .iter()
                            .any(|(j, c)| j == k && c.bone_node.is_some())
                    })
                    .collect();
                let mut count = 0;
                for mesh in r.mesh_targets(i) {
                    count = count.max(r.num_vertices(mesh)?);
                }
                if count
                    .checked_mul(
                        std::mem::size_of::<SkinVertex>() + std::mem::size_of::<Vec<SkinWeight>>(),
                    )
                    .is_none_or(|n| n > limits.max_allocation)
                {
                    return Err("skin: allocation limit".into());
                }
                r.charge(
                    count,
                    std::mem::size_of::<SkinVertex>() + std::mem::size_of::<Vec<SkinWeight>>(),
                )?;
                let mut per_vertex = vec![Vec::new(); count];
                for (ci, k) in clusters.iter().enumerate() {
                    let c = &d
                        .clusters
                        .iter()
                        .find(|(j, _)| j == k)
                        .ok_or("skin: missing cluster")?
                        .1;
                    for (&v, &w) in c.vertices.iter().zip(&c.weights) {
                        let dst = per_vertex
                            .get_mut(v as usize)
                            .ok_or("skin: vertex out of range")?;
                        dst.push(SkinWeight {
                            cluster_index: ci,
                            weight: w,
                        });
                    }
                }
                let dq_vertices = r.indices(n, b"Indexes")?;
                let dq_weights = r.reals(n, b"BlendWeights")?;
                if dq_vertices.len() != dq_weights.len()
                    || dq_vertices.iter().any(|&v| v as usize >= count)
                {
                    return Err("skin: invalid dual-quaternion weights".into());
                }
                let mut vertices = Vec::new();
                let mut weights = Vec::new();
                let mut max_weights = 0;
                for mut w in per_vertex {
                    w.sort_by(|a, b| {
                        b.weight
                            .partial_cmp(&a.weight)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    max_weights = max_weights.max(w.len());
                    vertices.push(SkinVertex {
                        weight_begin: weights.len(),
                        num_weights: w.len(),
                        dq_weight: if method == SkinningMethod::DualQuaternion {
                            1.0
                        } else {
                            0.0
                        },
                    });
                    weights.extend(w);
                }
                for (&v, &w) in dq_vertices.iter().zip(&dq_weights) {
                    vertices[v as usize].dq_weight = w;
                }
                d.skins.push((
                    i,
                    SkinDeformer {
                        name: r.name(i),
                        source_id: r.source_id(i),
                        skinning_method: method,
                        clusters,
                        vertices,
                        weights,
                        max_weights_per_vertex: max_weights,
                        dq_vertices,
                        dq_weights,
                    },
                ));
            }
            T::BlendDeformer => d.blends.push((
                i,
                BlendDeformer {
                    name: r.name(i),
                    source_id: r.source_id(i),
                    channels: r.dst(i, T::BlendChannel),
                },
            )),
            T::BlendChannel => {
                let shapes = r.dst(i, T::BlendShape);
                let full = r.reals(n, b"FullWeights")?;
                if s.exporter != 2 && !full.is_empty() && full.len() != shapes.len() {
                    return Err("blend channel: target weight count mismatch".into());
                }
                let props = if s.doc.nodes[n].name == b"Shape" {
                    crate::scene_property::PropertyLayers {
                        authored: Vec::new(),
                        defaults: None,
                    }
                } else {
                    crate::scene_property::layers(
                        &s.doc,
                        n,
                        limits.max_properties,
                        limits.max_allocation,
                    )
                    .map_err(|e| format!("blend properties: {e:?}"))?
                };
                let percent = if s.doc.nodes[n].name == b"Shape" {
                    let parent = s.doc.nodes[n].parent.ok_or("shape: missing geometry")?;
                    let p = crate::scene_property::layers(
                        &s.doc,
                        parent,
                        limits.max_properties,
                        limits.max_allocation,
                    )
                    .map_err(|e| format!("shape properties: {e:?}"))?;
                    p.find(&e.name)
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::scene_property::Type::Number
                                    | crate::scene_property::Type::Integer
                            )
                        })
                        .map_or(0.0, |p| p.value_real[0])
                } else if props.authored.is_empty() && r.child(n, b"DeformPercent").is_some() {
                    match r.data(n, b"DeformPercent") {
                        Some(NodeData::Values(v)) => match v.first() {
                            Some(Scalar::Num(_, x)) => *x,
                            _ => 100.0,
                        },
                        _ => 100.0,
                    }
                } else {
                    props
                        .find(b"DeformPercent")
                        .map_or(0.0, |p| p.value_real[0])
                };
                if !percent.is_finite() {
                    return Err("blend channel: invalid weight".into());
                }
                if s.exporter == 2 {
                    for &shape in &shapes {
                        if let Some((_, target)) = d.shapes.iter_mut().find(|(k, _)| *k == shape) {
                            if full.len() == target.offset_vertices.len() {
                                target.offset_weights = full.iter().map(|v| v / 100.0).collect();
                            }
                        }
                    }
                }
                let mut keys: Vec<BlendKeyframe> = shapes
                    .iter()
                    .enumerate()
                    .map(|(j, &shape)| BlendKeyframe {
                        shape,
                        target_weight: if s.exporter == 2 {
                            1.0
                        } else {
                            full.get(j).map_or(1.0, |w| w / 100.0)
                        },
                        effective_weight: 0.0,
                    })
                    .collect();
                keys.sort_by(|a, b| {
                    a.target_weight
                        .partial_cmp(&b.target_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let weight = percent * 0.01;
                effective_weights(&mut keys, weight);
                let target_shape = keys.last().map(|k| k.shape);
                d.channels.push((
                    i,
                    BlendChannel {
                        name: r.name(i),
                        source_id: r.source_id(i),
                        weight,
                        keyframes: keys,
                        target_shape,
                    },
                ));
            }
            _ => {}
        }
    }
    d.poses.sort_by_key(|(i, _)| *i);
    for (k, p) in node_bind.into_iter().enumerate() {
        if let Some(p) = p {
            s.elements[k].obj.set("bind_pose", V::Ref(Some(p)));
        }
    }
    if !s.lean {
        emit(s, &d);
    }
    s.deformers = d;
    Ok(())
}
fn effective_weights(keys: &mut [BlendKeyframe], weight: f64) {
    let last = keys.iter().rposition(|k| k.target_weight < 0.0);
    let mut prev = None;
    let mut next = None;
    if weight > 0.0 {
        for (i, k) in keys.iter().enumerate().skip(last.map_or(0, |i| i + 1)) {
            prev = next;
            next = Some(i);
            if k.target_weight > weight {
                break;
            }
        }
    } else {
        for i in (0..last.map_or(0, |i| i + 1)).rev() {
            prev = next;
            next = Some(i);
            if keys[i].target_weight < weight {
                break;
            }
        }
    }
    let a = prev.map_or(0.0, |i| keys[i].target_weight);
    let b = next.map_or(0.0, |i| keys[i].target_weight);
    if b - a != 0.0 {
        let t = (weight - a) / (b - a);
        if let Some(i) = prev {
            keys[i].effective_weight = 1.0 - t;
        }
        if let Some(i) = next {
            keys[i].effective_weight = t;
        }
    }
}
fn emit(s: &mut CanonScene<'_>, d: &Deformers) {
    for (i, c) in &d.clusters {
        let o = &mut s.elements[*i].obj;
        o.set("bone_node", V::Ref(c.bone_node));
        for (n, m) in [
            ("geometry_to_bone", c.geometry_to_bone),
            ("mesh_node_to_bone", c.mesh_node_to_bone),
            ("bind_to_world", c.bind_to_world),
            ("geometry_to_world", c.geometry_to_world),
        ] {
            o.set(n, V::Vec(m.0.to_vec()));
        }
        o.set(
            "geometry_to_world_transform",
            tr_obj(&c.geometry_to_world_transform),
        );
        o.set("num_weights", V::U(c.weights.len() as u64));
        o.set("vertices", ints(&c.vertices));
        o.set("weights", V::F64s(c.weights.clone()));
    }
    for (i, c) in &d.skins {
        let o = &mut s.elements[*i].obj;
        o.set(
            "skinning_method",
            V::I(match c.skinning_method {
                SkinningMethod::Linear => 0,
                SkinningMethod::Rigid => 1,
                SkinningMethod::DualQuaternion => 2,
                SkinningMethod::BlendedDqLinear => 3,
            }),
        );
        o.set("clusters", refs(&c.clusters));
        o.set(
            "vertices",
            V::List(
                c.vertices
                    .iter()
                    .map(|v| {
                        V::Obj(
                            Obj::new()
                                .with("weight_begin", V::U(v.weight_begin as u64))
                                .with("num_weights", V::U(v.num_weights as u64))
                                .with("dq_weight", V::F(v.dq_weight)),
                        )
                    })
                    .collect(),
            ),
        );
        o.set(
            "weights",
            V::List(
                c.weights
                    .iter()
                    .map(|v| {
                        V::Obj(
                            Obj::new()
                                .with("cluster_index", V::U(v.cluster_index as u64))
                                .with("weight", V::F(v.weight)),
                        )
                    })
                    .collect(),
            ),
        );
        o.set(
            "max_weights_per_vertex",
            V::U(c.max_weights_per_vertex as u64),
        );
        o.set("num_dq_weights", V::U(c.dq_weights.len() as u64));
        o.set("dq_vertices", ints(&c.dq_vertices));
        o.set("dq_weights", V::F64s(c.dq_weights.clone()));
    }
    for (i, c) in &d.blends {
        s.elements[*i].obj.set("channels", refs(&c.channels));
    }
    for (i, c) in &d.channels {
        let o = &mut s.elements[*i].obj;
        o.set("weight", V::F(c.weight));
        o.set("target_shape", V::Ref(c.target_shape));
        o.set(
            "keyframes",
            V::List(
                c.keyframes
                    .iter()
                    .map(|k| {
                        V::Obj(
                            Obj::new()
                                .with("shape", V::Ref(Some(k.shape)))
                                .with("target_weight", V::F(k.target_weight))
                                .with("effective_weight", V::F(k.effective_weight)),
                        )
                    })
                    .collect(),
            ),
        );
    }
    for (i, c) in &d.shapes {
        let o = &mut s.elements[*i].obj;
        o.set("num_offsets", V::U(c.offset_vertices.len() as u64));
        o.set("offset_vertices", ints(&c.offset_vertices));
        o.set("position_offsets", vecs(&c.position_offsets));
        o.set("normal_offsets", vecs(&c.normal_offsets));
        o.set("offset_weights", V::F64s(c.offset_weights.clone()));
    }
    for (i, c) in &d.poses {
        let o = &mut s.elements[*i].obj;
        o.set("is_bind_pose", V::Bool(c.is_bind_pose));
        o.set(
            "bone_poses",
            V::List(
                c.bone_poses
                    .iter()
                    .map(|b| {
                        V::Obj(
                            Obj::new()
                                .with("bone_node", V::Ref(Some(b.bone_node)))
                                .with("bone_to_world", V::Vec(b.bone_to_world.0.to_vec()))
                                .with("bone_to_parent", V::Vec(b.bone_to_parent.0.to_vec())),
                        )
                    })
                    .collect(),
            ),
        );
    }
}
