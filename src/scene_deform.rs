//! Static deformer conversion following pinned ufbx 0.23.1:
//! read_skin(_cluster), read_shape, finalize_scene, update_initial_clusters,
//! update_pose, update_skin_cluster and update_blend_channel.
use crate::canon::{Obj, V};
use crate::canon_dom::{data_len, node_data, NodeData, Scalar};
use crate::canon_scene::{CanonScene, DumpLimits, ElementType as T};
use crate::deform::*;
use crate::fallible_map::{Map as BTreeMap, Set as BTreeSet};
use crate::scene_node as sn;
use crate::Error;
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
    allocation: crate::api_alloc::Budget,
    children: Vec<Vec<usize>>,
    used: std::cell::Cell<usize>,
    limit: usize,
    work: std::cell::Cell<usize>,
    work_limit: usize,
    path: std::cell::Cell<crate::conversion_budget::Path>,
}
impl Reader<'_, '_> {
    fn new<'a, 'd>(s: &'a CanonScene<'d>, limits: &DumpLimits) -> Result<Reader<'a, 'd>, Error> {
        let mut r = Reader {
            s,
            allocation: crate::api_alloc::Budget::new(limits.max_allocation),
            children: Vec::new(),
            used: std::cell::Cell::new(0),
            limit: limits.max_allocation,
            work: std::cell::Cell::new(0),
            work_limit: limits.max_deformer_work,
            path: std::cell::Cell::new(crate::conversion_budget::Path::Deformers),
        };
        // Outer adjacency, child counts, and at most twice the child entries
        // for Vec growth, before constructing any of the adjacency table.
        r.charge(s.doc.nodes.len(), std::mem::size_of::<Vec<usize>>() + 32)?;
        r.work(s.doc.nodes.len())?;
        r.children = r.allocation.filled(s.doc.nodes.len(), Vec::new())?;
        for (i, n) in s.doc.nodes.iter().enumerate() {
            if let Some(p) = n.parent {
                r.allocation.push(&mut r.children[p], i)?;
            }
        }
        Ok(r)
    }
    fn work(&self, count: usize) -> Result<(), Error> {
        let used = self
            .work
            .get()
            .checked_add(count)
            .ok_or(crate::Error::LimitExceeded(crate::Limit::DeformerWork))?;
        if used > self.work_limit {
            return Err(crate::Error::LimitExceeded(crate::Limit::DeformerWork));
        }
        self.work.set(used);
        crate::conversion_budget::record(self.path.get(), count);
        Ok(())
    }
    fn sort_work(&self, count: usize) -> Result<(), Error> {
        self.work(
            count
                .checked_mul((usize::BITS - count.leading_zeros()) as usize + 1)
                .ok_or(crate::Error::LimitExceeded(crate::Limit::DeformerWork))?,
        )
    }
    fn charge(&self, count: usize, width: usize) -> Result<(), Error> {
        let used = count
            .checked_mul(width)
            .and_then(|n| self.used.get().checked_add(n))
            .ok_or(crate::Error::LimitExceeded(crate::Limit::Allocation))?;
        if used > self.limit {
            return Err(crate::Error::LimitExceeded(crate::Limit::Allocation));
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
    fn data(&self, n: usize, name: &[u8]) -> Result<Option<NodeData>, Error> {
        self.work(self.children[n].len())?;
        let Some(c) = self.child(n, name) else {
            return Ok(None);
        };
        let count = data_len(&self.s.doc, c)?;
        // Covers the ASCII token/number/encoded-byte temporaries, decoded
        // array, derived offsets/influences and stable-sort scratch. Charge
        // from the source count BEFORE node_data materializes any of them.
        self.charge(count, 128)?;
        for p in &self.s.doc.properties[self.s.doc.nodes[c].properties.clone()] {
            if let crate::syntax::Property::String(v) = p {
                self.charge(v.len(), 8)?;
            }
        }
        self.work(count)?;
        Ok(Some(node_data(
            &self.s.doc,
            c,
            self.s.ascii,
            &self.allocation,
        )?))
    }
    fn reals(&self, n: usize, name: &[u8]) -> Result<Vec<f64>, Error> {
        match self.data(n, name)? {
            None => Ok(Vec::new()),
            Some(NodeData::Reals(v)) if v.iter().all(|x| x.is_finite()) => Ok(v),
            _ => Err(Error::InvalidData("deformer: invalid real array")),
        }
    }
    fn indices(&self, n: usize, name: &[u8]) -> Result<Vec<u32>, Error> {
        match self.data(n, name)? {
            None => Ok(Vec::new()),
            Some(NodeData::Ints(v)) if v.iter().all(|&x| x >= 0) => self
                .allocation
                .collect(v.into_iter().map(|x| u32::try_from(x).unwrap())),
            _ => Err(Error::InvalidData("deformer: invalid index array")),
        }
    }
    fn matrix(&self, n: usize, name: &[u8]) -> Result<sn::Mat, Error> {
        let v = self.reals(n, name)?;
        if v.is_empty() && self.child(n, name).is_none() {
            return Ok([0.0; 12]);
        }
        if v.len() != 16 {
            return Err("deformer: matrix must have 16 values".into());
        }
        Ok(std::array::from_fn(|i| v[i / 3 * 4 + i % 3]))
    }
    fn connected(&self, conns: &[(usize, Vec<u8>)], ty: T) -> Result<Vec<usize>, Error> {
        self.sort_work(conns.len())?;
        self.charge(conns.len(), 80)?;
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for (k, p) in conns {
            if p.is_empty()
                && self.s.elements[*k].ty == ty
                && seen.try_insert(*k, &self.allocation)?
            {
                self.allocation.push(&mut out, *k)?;
            }
        }
        Ok(out)
    }
    fn dst(&self, i: usize, ty: T) -> Result<Vec<usize>, Error> {
        self.connected(&self.s.elements[i].dst_conns, ty)
    }
    fn src(&self, i: usize, ty: T) -> Result<Vec<usize>, Error> {
        self.connected(&self.s.elements[i].src_conns, ty)
    }
    fn mesh_targets(&self, i: usize) -> Result<Vec<usize>, Error> {
        let mut out = self.src(i, T::Mesh)?;
        for node in self.src(i, T::Node)? {
            let meshes = self.dst(node, T::Mesh)?;
            self.charge(meshes.len(), 16)?;
            self.allocation.reserve(&mut out, meshes.len())?;
            out.extend(meshes);
        }
        Ok(out)
    }
    fn num_vertices(&self, mesh: usize) -> Result<usize, Error> {
        let n = self.s.elements[mesh]
            .node
            .ok_or("deformer: mesh has no source")?;
        self.work(self.children[n].len())?;
        // Mesh conversion already validates these values. Count without
        // copying the whole mesh again for every cluster/shape.
        self.child(n, b"Vertices")
            .map_or(Ok(0), |c| data_len(&self.s.doc, c).map(|n| n / 3))
    }
    fn source_id(&self, i: usize) -> Option<i64> {
        let e = &self.s.elements[i];
        if e.node.is_some_and(|n| self.s.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        }
    }
    fn name(&self, i: usize) -> Result<String, Error> {
        self.charge(self.s.elements[i].name.len(), 3)?;
        self.allocation.text(&self.s.elements[i].name)
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
    let a = usize::try_from(axis).expect("validated mirror axis") - 1;
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
pub(crate) fn project(s: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), Error> {
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
    let r = Reader::new(s, limits)?;
    let a = &r.allocation;
    // Element-indexed maps, traversal stacks and typed record headers.
    r.charge(s.elements.len(), 1024)?;
    r.work(s.elements.len())?;
    // Reject deformer-only cycles, including unconnected cycles, without recursion.
    let mut color = a.filled(s.elements.len(), 0u8)?;
    for start in 0..s.elements.len() {
        if !matches!(
            s.elements[start].ty,
            T::SkinDeformer | T::SkinCluster | T::BlendDeformer | T::BlendChannel | T::BlendShape
        ) || color[start] != 0
        {
            continue;
        }
        let mut stack = a.collect([(start, 0)])?;
        color[start] = 1;
        while let Some((i, next)) = stack.last_mut() {
            let conns = &s.elements[*i].src_conns;
            if *next == conns.len() {
                color[*i] = 2;
                stack.pop();
                continue;
            }
            let (k, p) = &conns[*next];
            r.work(1)?;
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
                a.push(&mut stack, (*k, 0))?;
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
    let mut node_bind = a.filled(s.elements.len(), None)?;
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
    r.path.set(crate::conversion_budget::Path::Poses);
    let mut by_source = BTreeMap::new();
    r.sort_work(s.elements.len())?;
    for (k, e) in s.elements.iter().enumerate() {
        if e.ty == T::Node {
            if let Some(id) = e.source_id {
                by_source.try_get_or_insert_with(id, a, || Ok(k))?;
            }
        }
    }
    let mut by_name = a.collect(
        s.elements
            .iter()
            .enumerate()
            .take(if s.doc.version < 7000 {
                s.elements.len()
            } else {
                0
            })
            .filter_map(|(i, e)| {
                if e.ty != T::Node {
                    return None;
                }
                e.node.map(|n| {
                    (
                        crate::legacy::string(
                            s.doc.properties[s.doc.nodes[n].properties.clone()].first(),
                        ),
                        i,
                    )
                })
            }),
    )?;
    r.sort_work(by_name.len())?;
    by_name.sort_unstable();
    // Memoize parent depth once. Each node is walked at most twice, even
    // for deep chains and multiple poses; sorting never walks ancestors.
    let mut depths: Vec<Option<usize>> = a.filled(s.elements.len(), None)?;
    let mut visiting = a.filled(s.elements.len(), false)?;
    let mut path = Vec::new();
    for start in 0..s.elements.len() {
        if s.elements[start].ty != T::Node || depths[start].is_some() {
            continue;
        }
        let mut at = Some(start);
        while let Some(k) = at {
            r.work(1)?;
            if depths[k].is_some() {
                break;
            }
            if visiting[k] {
                return Err("pose: parent cycle".into());
            }
            visiting[k] = true;
            a.push(&mut path, k)?;
            at = parent_of(k);
        }
        let depth = at.and_then(|k| depths[k]).map_or(0, |d| d + 1);
        for (depth, k) in (depth..).zip(path.drain(..).rev()) {
            depths[k] = Some(depth);
            visiting[k] = false;
        }
    }
    let mut first_bind = a.filled(s.elements.len(), None)?;
    let mut pose_bone = a.filled(s.elements.len(), None)?;
    let mut pose_ids: Vec<usize> = a.collect(
        s.elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.ty == T::Pose)
            .map(|(i, _)| i),
    )?;
    r.sort_work(pose_ids.len())?;
    a.stable_sort(&mut pose_ids, |left, right| {
        let key = |&i: &usize| s.elements[i].ordinal;
        key(left).cmp(&key(right))
    })?;
    for i in pose_ids {
        let e = &s.elements[i];
        let n = e.node.ok_or("pose: missing source")?;
        let props = &s.doc.properties[s.doc.nodes[n].properties.clone()];
        let bind = matches!(props.get(if s.doc.version < 7000 { 1 } else { 2 }),Some(crate::syntax::Property::String(v)) if v.as_ref()==b"BindPose");
        let mut bones = Vec::new();
        r.charge(r.children[n].len(), std::mem::size_of::<BonePose>() * 3)?;
        r.work(r.children[n].len())?;
        for &pn in &r.children[n] {
            if s.doc.nodes[pn].name != b"PoseNode" {
                continue;
            }
            let k = if s.doc.version < 7000 {
                let c = r.child(pn, b"Node").ok_or("pose: missing node")?;
                let nm = crate::legacy::string(
                    s.doc.properties[s.doc.nodes[c].properties.clone()].first(),
                );
                r.work((usize::BITS - by_name.len().leading_zeros()) as usize + 1)?;
                let k = by_name
                    .binary_search_by(|p| p.0.cmp(nm))
                    .map_err(|_| "pose: invalid node name")?;
                by_name[k].1
            } else {
                let id = match r.data(pn, b"Node")? {
                    Some(NodeData::Values(v)) => match v.first() {
                        Some(Scalar::Num(x, _)) => *x,
                        _ => return Err("pose: invalid node".into()),
                    },
                    _ => return Err("pose: missing node".into()),
                };
                r.work((usize::BITS - by_source.len().leading_zeros()) as usize + 1)?;
                by_source.get(&id).copied().ok_or("pose: invalid node ID")?
            };
            if r.child(pn, b"Matrix").is_none() {
                return Err("pose: missing matrix".into());
            }
            let m = convert_bind(r.matrix(pn, b"Matrix")?);
            if bind && node_bind[k].is_none() {
                node_bind[k] = Some(i);
                first_bind[k] = Some(m);
            }
            a.push(
                &mut bones,
                BonePose {
                    bone_node: k,
                    bone_to_world: Matrix(m),
                    bone_to_parent: Matrix([0.0; 12]),
                },
            )?;
        }
        // ufbx typed node order: depth, original parent element ID, original ID.
        r.sort_work(bones.len())?;
        a.stable_sort(&mut bones, |left, right| {
            let key = |b: &BonePose| {
                (
                    depths[b.bone_node],
                    parent_of(b.bone_node).map(|k| s.elements[k].ordinal),
                    s.elements[b.bone_node].ordinal,
                )
            };
            key(left).cmp(&key(right))
        })?;
        for (j, b) in bones.iter().enumerate() {
            // Preserve the first entry when malformed poses repeat a node.
            pose_bone[b.bone_node].get_or_insert(j);
        }
        r.work(bones.len())?;
        for j in 0..bones.len() {
            let parent = parent_of(bones[j].bone_node);
            let pm = parent
                .and_then(|p| {
                    pose_bone[p]
                        .map(|j| bones[j].bone_to_world.0)
                        .or_else(|| s.node_states[p].as_ref().map(|st| st.node_to_world))
                })
                .unwrap_or(sn::IDENTITY_MATRIX);
            bones[j].bone_to_parent = Matrix(sn::matrix_mul(
                &sn::matrix_invert(&pm),
                &bones[j].bone_to_world.0,
            ));
        }
        for b in &bones {
            pose_bone[b.bone_node] = None;
        }
        a.push(
            &mut d.poses,
            (
                i,
                Pose {
                    name: r.name(i)?,
                    source_id: r.source_id(i),
                    is_bind_pose: bind,
                    bone_poses: bones,
                },
            ),
        )?;
    }
    r.path.set(crate::conversion_budget::Path::Deformers);
    let mut cluster_index = a.filled(s.elements.len(), None)?;
    let mut shape_index = a.filled(s.elements.len(), None)?;
    let mut shape_orders = Vec::new();
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
                for channel in r.src(i, T::BlendChannel)? {
                    for blend in r.src(channel, T::BlendDeformer)? {
                        for mesh in r.mesh_targets(blend)? {
                            let count = r.num_vertices(mesh)?;
                            r.work(indices.len())?;
                            if indices.iter().any(|&v| v as usize >= count) {
                                return Err("blend shape: vertex out of range".into());
                            }
                        }
                    }
                }
                r.sort_work(indices.len())?;
                let mut order: Vec<usize> = a.collect(0..indices.len())?;
                a.stable_sort(&mut order, |left, right| {
                    let key = |&j: &usize| indices[j];
                    key(left).cmp(&key(right))
                })?;
                let offsets = |values: &[f64], scale: f64| -> Result<Vec<[f64; 3]>, Error> {
                    if values.is_empty() {
                        return Ok(Vec::new());
                    }
                    a.collect(order.iter().map(|&j| {
                        let mut v = [values[j * 3], values[j * 3 + 1], values[j * 3 + 2]];
                        if scale != 1.0 {
                            for x in &mut v {
                                *x *= scale;
                            }
                        }
                        if axis != 0 {
                            v[usize::try_from(axis).expect("validated mirror axis") - 1] =
                                -v[usize::try_from(axis).expect("validated mirror axis") - 1];
                        }
                        v
                    }))
                };
                shape_index[i] = Some(d.shapes.len());
                a.push(
                    &mut d.shapes,
                    (
                        i,
                        BlendShape {
                            name: r.name(i)?,
                            source_id: r.source_id(i),
                            offset_vertices: a.collect(order.iter().map(|&j| indices[j]))?,
                            position_offsets: offsets(&pos, scale)?,
                            normal_offsets: offsets(&normal, 1.0)?,
                            offset_weights: Vec::new(),
                        },
                    ),
                )?;
                a.push(&mut shape_orders, order)?;
            }
            T::SkinCluster => {
                let vertices = r.indices(n, b"Indexes")?;
                let weights = r.reals(n, b"Weights")?;
                if vertices.len() != weights.len() {
                    return Err("skin cluster: weight count mismatch".into());
                }
                let bone = r.dst(i, T::Node)?.first().copied();
                let mut mesh_mat = r.matrix(n, b"Transform")?;
                let mut bind = r.matrix(n, b"TransformLink")?;
                if bind.iter().all(|&x| x == 0.0) {
                    if let Some(k) = bone {
                        if let Some(m) = first_bind[k] {
                            bind = m;
                        }
                    }
                } else {
                    bind = convert_bind(bind);
                }
                let mut geom = mesh_mat;
                if let Some(skin) = r.src(i, T::SkinDeformer)?.first().copied() {
                    let meshes = r.mesh_targets(skin)?;
                    for &mesh in &meshes {
                        let count = r.num_vertices(mesh)?;
                        r.work(vertices.len())?;
                        if vertices.iter().any(|&v| v as usize >= count) {
                            return Err("skin cluster: vertex out of range".into());
                        }
                    }
                    let node = r.src(skin, T::Node)?.first().copied().or_else(|| {
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
                cluster_index[i] = Some(d.clusters.len());
                a.push(
                    &mut d.clusters,
                    (
                        i,
                        SkinCluster {
                            name: r.name(i)?,
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
                    ),
                )?;
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
                let method = match r.data(n, b"SkinningType")? {
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
                let clusters: Vec<usize> =
                    a.collect(r.dst(i, T::SkinCluster)?.into_iter().filter(|&k| {
                        cluster_index[k].is_some_and(|j| d.clusters[j].1.bone_node.is_some())
                    }))?;
                let mut count = 0;
                for mesh in r.mesh_targets(i)? {
                    count = count.max(r.num_vertices(mesh)?);
                }
                if count
                    .checked_mul(
                        std::mem::size_of::<SkinVertex>() + std::mem::size_of::<Vec<SkinWeight>>(),
                    )
                    .is_none_or(|n| n > limits.max_allocation)
                {
                    return Err(crate::Error::LimitExceeded(crate::Limit::Allocation));
                }
                r.charge(
                    count,
                    std::mem::size_of::<SkinVertex>() + std::mem::size_of::<Vec<SkinWeight>>(),
                )?;
                let mut per_vertex = a.filled(count, Vec::new())?;
                for (ci, k) in clusters.iter().enumerate() {
                    let c = &d.clusters[cluster_index[*k].ok_or("skin: missing cluster")?].1;
                    r.charge(c.weights.len(), std::mem::size_of::<SkinWeight>() * 8)?;
                    r.sort_work(c.weights.len())?;
                    for (&v, &w) in c.vertices.iter().zip(&c.weights) {
                        let dst = per_vertex
                            .get_mut(v as usize)
                            .ok_or("skin: vertex out of range")?;
                        a.push(
                            dst,
                            SkinWeight {
                                cluster_index: ci,
                                weight: w,
                            },
                        )?;
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
                a.reserve(&mut vertices, count)?;
                let mut weights = Vec::new();
                let mut max_weights = 0;
                for mut w in per_vertex {
                    r.sort_work(w.len())?;
                    a.stable_sort(&mut w, |a, b| {
                        b.weight
                            .partial_cmp(&a.weight)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })?;
                    max_weights = max_weights.max(w.len());
                    a.push(
                        &mut vertices,
                        SkinVertex {
                            weight_begin: weights.len(),
                            num_weights: w.len(),
                            dq_weight: if method == SkinningMethod::DualQuaternion {
                                1.0
                            } else {
                                0.0
                            },
                        },
                    )?;
                    a.reserve(&mut weights, w.len())?;
                    weights.extend(w);
                }
                for (&v, &w) in dq_vertices.iter().zip(&dq_weights) {
                    vertices[v as usize].dq_weight = w;
                }
                a.push(
                    &mut d.skins,
                    (
                        i,
                        SkinDeformer {
                            name: r.name(i)?,
                            source_id: r.source_id(i),
                            skinning_method: method,
                            clusters,
                            vertices,
                            weights,
                            max_weights_per_vertex: max_weights,
                            dq_vertices,
                            dq_weights,
                        },
                    ),
                )?;
            }
            T::BlendDeformer => a.push(
                &mut d.blends,
                (
                    i,
                    BlendDeformer {
                        name: r.name(i)?,
                        source_id: r.source_id(i),
                        channels: r.dst(i, T::BlendChannel)?,
                    },
                ),
            )?,
            T::BlendChannel => {
                let shapes = r.dst(i, T::BlendShape)?;
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
                    crate::scene_property::layers_indexed(
                        &s.doc,
                        &s.props,
                        n,
                        limits.max_properties,
                        limits.max_allocation,
                    )
                    .map_err(crate::api::property_error)?
                };
                let percent = if s.doc.nodes[n].name == b"Shape" {
                    let parent = s.doc.nodes[n].parent.ok_or("shape: missing geometry")?;
                    let p = crate::scene_property::layers_indexed(
                        &s.doc,
                        &s.props,
                        parent,
                        limits.max_properties,
                        limits.max_allocation,
                    )
                    .map_err(crate::api::property_error)?;
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
                    match r.data(n, b"DeformPercent")? {
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
                        if let Some(j) = shape_index[shape] {
                            let target = &mut d.shapes[j].1;
                            if full.len() == target.offset_vertices.len() {
                                r.charge(full.len(), 8)?;
                                r.work(full.len())?;
                                target.offset_weights =
                                    a.collect(shape_orders[j].iter().map(|&k| full[k] / 100.0))?;
                            }
                        }
                    }
                }
                r.charge(shapes.len(), std::mem::size_of::<BlendKeyframe>() * 3)?;
                let mut keys: Vec<BlendKeyframe> =
                    a.collect(shapes.iter().enumerate().map(|(j, &shape)| BlendKeyframe {
                        shape,
                        target_weight: if s.exporter == 2 {
                            1.0
                        } else {
                            full.get(j).map_or(1.0, |w| w / 100.0)
                        },
                        effective_weight: 0.0,
                    }))?;
                r.sort_work(keys.len())?;
                a.stable_sort(&mut keys, |a, b| {
                    a.target_weight
                        .partial_cmp(&b.target_weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })?;
                let weight = percent * 0.01;
                effective_weights(&mut keys, weight);
                let target_shape = keys.last().map(|k| k.shape);
                a.push(
                    &mut d.channels,
                    (
                        i,
                        BlendChannel {
                            name: r.name(i)?,
                            source_id: r.source_id(i),
                            weight,
                            keyframes: keys,
                            target_shape,
                        },
                    ),
                )?;
            }
            _ => {}
        }
    }
    r.sort_work(d.poses.len())?;
    a.stable_sort(&mut d.poses, |left, right| {
        let key = |(i, _): &(usize, Pose)| *i;
        key(left).cmp(&key(right))
    })?;
    let allocation = r.allocation;
    let a = &allocation;
    for (k, p) in node_bind.into_iter().enumerate() {
        if let Some(p) = p {
            s.elements[k].obj.try_set("bind_pose", V::Ref(Some(p)), a)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Property;

    const INPUT: &[u8] = b"; FBX 7.4.0\nObjects: { Deformer: 1, \"Deformer::C\", \"Cluster\" { Indexes: *1 { a: 0 }\nWeights: *1 { a: 1 }\n}\n}\n";

    #[test]
    fn binary_wide_sparse_indices_are_checked_in_both_byte_orders() {
        for be in [false, true] {
            for value in [
                0i64,
                i32::MAX as i64,
                4294967296,
                -4294967296,
                i64::MAX,
                i64::MIN,
            ] {
                let mut s = crate::canon_scene::project(INPUT, &DumpLimits::default()).unwrap();
                let n = s
                    .doc
                    .nodes
                    .iter()
                    .position(|n| n.name == b"Indexes")
                    .unwrap();
                let p = s.doc.nodes[n].properties.start;
                let bytes = if be {
                    value.to_be_bytes()
                } else {
                    value.to_le_bytes()
                };
                s.doc.properties[p] = Property::Array {
                    kind: b'l',
                    count: 1,
                    big_endian: be,
                    bytes: bytes.to_vec().into(),
                };
                s.ascii = false;
                let r = Reader::new(&s, &DumpLimits::default()).unwrap();
                let result = r.indices(s.doc.nodes[n].parent.unwrap(), b"Indexes");
                if (0..=i32::MAX as i64).contains(&value) {
                    assert_eq!(result.unwrap(), [u32::try_from(value).unwrap()]);
                } else {
                    assert!(result.unwrap_err().to_string().contains("invalid index"));
                }
            }
        }
    }

    #[test]
    fn adjacency_and_arrays_are_charged_before_materializing() {
        let mut s = crate::canon_scene::project(INPUT, &DumpLimits::default()).unwrap();
        let limits = DumpLimits {
            max_allocation: 0,
            ..Default::default()
        };
        assert!(
            matches!(Reader::new(&s, &limits), Err(e) if e.to_string().contains("allocation limit"))
        );
        let n = s
            .doc
            .nodes
            .iter()
            .position(|n| n.name == b"Indexes")
            .unwrap();
        let p = s.doc.nodes[n].properties.start;
        // Impossible array payload: preflight must fail on budget before a
        // decoder can discard it as empty or try to materialize it.
        s.doc.properties[p] = Property::Array {
            kind: b'l',
            count: usize::MAX,
            big_endian: false,
            bytes: Vec::new().into(),
        };
        let r = Reader::new(&s, &DumpLimits::default()).unwrap();
        assert!(r
            .indices(s.doc.nodes[n].parent.unwrap(), b"Indexes")
            .unwrap_err()
            .to_string()
            .contains("allocation limit"));
        let weights = s
            .doc
            .nodes
            .iter()
            .position(|n| n.name == b"Weights")
            .unwrap();
        let r = Reader::new(&s, &DumpLimits::default()).unwrap();
        r.used.set(r.limit);
        assert!(r
            .reals(s.doc.nodes[weights].parent.unwrap(), b"Weights")
            .unwrap_err()
            .to_string()
            .contains("allocation limit"));
    }
}
