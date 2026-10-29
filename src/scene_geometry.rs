//! `ufbx_mesh` records of the engine-converted profile, ported operation for
//! operation from pinned ufbx 0.23.1 (`ufbx.c`, `ufbx_real` = `double`,
//! reference build `-ffp-contract=off`).
//!
//! Load order reproduced per mesh: `ufbxi_read_mesh` (with
//! `ufbxi_read_vertex_element`, `ufbxi_read_truncated_array`,
//! `ufbxi_process_indices`, `ufbxi_assign_face_groups`), the mesh part of
//! `ufbxi_finalize_scene` (`ufbxi_generate_normals`, material lookup,
//! `ufbxi_finalize_mesh_material`, deformer lists), `ufbxi_modify_geometry`
//! (unit scale, handedness mirror and winding flip, geometric transforms) and
//! `ufbxi_postprocess_scene` (normal normalization). Index-error handling is
//! `UFBX_INDEX_ERROR_HANDLING_CLAMP`.
use crate::canon::{s, Obj, V};
use crate::canon_dom::{node_bools, node_data, NodeData, Scalar};
use crate::scene_node::{self as sn, Mat};
use crate::syntax::Document;

const NO_INDEX: u32 = u32::MAX;
/// `UFBX_EPSILON` for double `ufbx_real`.
const EPSILON: f64 = 1.491_668_146_240_041_3e-154;

/// Scene facts one mesh record depends on.
pub struct MeshEnv<'a, 'd> {
    pub doc: &'a Document<'d>,
    /// DOM children per DOM node index.
    pub children: &'a [Vec<usize>],
    pub ascii: bool,
    /// `ufbx_metadata.geometry_scale`.
    pub geometry_scale: f64,
    /// `ufbx_metadata.mirror_axis` (0 none, 1..3 X..Z).
    pub mirror_axis: i64,
}

/// Element links of one mesh, already in ufbx order (element indices).
#[derive(Debug, Default)]
pub struct MeshLinks {
    pub materials: Vec<usize>,
    pub skin_deformers: Vec<usize>,
    pub blend_deformers: Vec<usize>,
    pub cache_deformers: Vec<usize>,
    pub all_deformers: Vec<usize>,
    /// `geometry_to_node` of the single instance when it has a geometry transform.
    pub geometry_to_node: Option<Mat>,
}

/// `ufbx_vertex_attrib` with flat `ufbx_real` values.
#[derive(Debug, Clone, Default)]
struct Attrib {
    exists: bool,
    values: Vec<f64>,
    indices: Vec<u32>,
    unique_per_vertex: bool,
}

#[derive(Debug, Clone, Default)]
struct Part {
    index: u32,
    num_faces: u64,
    num_triangles: u64,
    num_bad: [u64; 3],
    face_indices: Vec<u32>,
}
impl Part {
    /// `ufbxi_mesh_part_add_face`.
    fn add_face(&mut self, num_indices: u32) {
        self.num_faces += 1;
        if num_indices >= 3 {
            self.num_triangles += u64::from(num_indices - 2);
        } else {
            self.num_bad[num_indices as usize] += 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct UvSet {
    name: Vec<u8>,
    index: u32,
    uv: Attrib,
    /// Tangent/bitangent layers (indices into `Mesh::tangent_layers`).
    tangent: Option<usize>,
    bitangent: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct ColorSet {
    name: Vec<u8>,
    index: u32,
    color: Attrib,
}

#[derive(Debug, Default)]
pub(crate) struct Mesh {
    num_vertices: usize,
    num_indices: usize,
    faces: Vec<(u32, u32)>,
    num_triangles: u64,
    pub(crate) max_face_triangles: u64,
    num_bad: [u64; 3],
    face_smoothing: Option<Vec<bool>>,
    face_material: Option<Vec<u32>>,
    face_group: Option<Vec<u32>>,
    face_hole: Option<Vec<bool>>,
    edges: Option<Vec<[u32; 2]>>,
    edge_smoothing: Option<Vec<bool>>,
    edge_crease: Option<Vec<f64>>,
    edge_visibility: Option<Vec<bool>>,
    vertex_first_index: Vec<u32>,
    position: Attrib,
    normal: Attrib,
    crease: Attrib,
    uv_sets: Vec<UvSet>,
    color_sets: Vec<ColorSet>,
    /// Tangent and bitangent layer storage (shared by reference from UV sets).
    tangent_layers: Vec<Attrib>,
    face_groups: Vec<i32>,
    face_group_parts: Vec<Part>,
    material_parts: Vec<Part>,
    material_part_usage_order: Vec<u32>,
    skinned_is_local: bool,
    subdivision_preview_levels: u64,
    subdivision_render_levels: u64,
    subdivision_display_mode: i64,
    subdivision_boundary: i64,
    reversed_winding: bool,
    generated_normals: bool,
}

fn err(what: &str) -> String {
    format!("mesh: {what}")
}

impl MeshEnv<'_, '_> {
    fn child(&self, node: usize, name: &[u8]) -> Option<usize> {
        self.children[node]
            .iter()
            .copied()
            .find(|&c| self.doc.nodes[c].name == name)
    }
    fn reals(&self, node: usize) -> Option<Vec<f64>> {
        match node_data(self.doc, node, self.ascii) {
            NodeData::Reals(v) => Some(v),
            _ => None,
        }
    }
    fn ints(&self, node: usize) -> Option<Vec<i32>> {
        match node_data(self.doc, node, self.ascii) {
            NodeData::Ints(v) => Some(v),
            _ => None,
        }
    }
    fn find_reals(&self, node: usize, name: &[u8]) -> Option<Vec<f64>> {
        self.child(node, name).and_then(|c| self.reals(c))
    }
    fn find_ints(&self, node: usize, name: &[u8]) -> Option<Vec<i32>> {
        self.child(node, name).and_then(|c| self.ints(c))
    }
    fn find_bools(&self, node: usize, name: &[u8]) -> Option<Vec<bool>> {
        self.child(node, name)
            .and_then(|c| node_bools(self.doc, c, self.ascii))
    }
    /// `ufbxi_get_val1(node, "I")`.
    fn val_i32(&self, node: usize) -> Option<i32> {
        match node_data(self.doc, node, self.ascii) {
            NodeData::Values(v) => match v.first() {
                Some(Scalar::Num(i, _)) => i32::try_from(*i).ok(),
                _ => None,
            },
            _ => None,
        }
    }
    /// `ufbxi_get_val1(node, "S"/"C")`.
    fn val_str(&self, node: usize) -> Option<Vec<u8>> {
        match node_data(self.doc, node, self.ascii) {
            NodeData::Values(v) => match v.into_iter().next() {
                Some(Scalar::Str(s)) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }
    fn find_i32(&self, node: usize, name: &[u8]) -> Option<i32> {
        self.child(node, name).and_then(|c| self.val_i32(c))
    }
    fn find_str(&self, node: usize, name: &[u8]) -> Option<Vec<u8>> {
        self.child(node, name).and_then(|c| self.val_str(c))
    }
}

/// `ufbxi_fix_index` under CLAMP.
fn fix_index(one_past_max: usize) -> Result<u32, String> {
    if one_past_max == 0 || one_past_max > u32::MAX as usize {
        return Err(err("clamped index without elements"));
    }
    u32::try_from(one_past_max - 1).map_err(|_| err("index count overflow"))
}

/// `ufbxi_check_indices`: pad with NO_INDEX, clamp out-of-range values.
fn check_indices(
    mut indices: Vec<u32>,
    num_indexers: usize,
    num_elems: usize,
) -> Result<Vec<u32>, String> {
    if indices.len() < num_indexers {
        indices.resize(num_indexers, NO_INDEX);
    }
    for ix in indices.iter_mut() {
        if *ix as usize >= num_elems {
            *ix = fix_index(num_elems)?;
        }
    }
    indices.truncate(num_indexers);
    Ok(indices)
}

/// `ufbxi_read_truncated_array`: exactly `size` elements, padded with the last.
fn truncated<T: Copy + Default>(arr: Option<Vec<T>>, size: usize) -> Option<Vec<T>> {
    let mut v = arr?;
    if v.len() < size {
        let fill = v.last().copied().unwrap_or_default();
        v.resize(size, fill);
    }
    v.truncate(size);
    Some(v)
}

fn mapping_is(m: &[u8], name: &str) -> bool {
    m == name.as_bytes()
}

impl Mesh {
    /// `ufbxi_read_vertex_element`.
    #[allow(clippy::too_many_arguments)]
    fn read_vertex_element(
        &self,
        env: &MeshEnv<'_, '_>,
        node: usize,
        attrib: &mut Attrib,
        data_name: &[u8],
        index_name: &[u8],
        comps: usize,
    ) -> Result<(), String> {
        let Some(data) = env.find_reals(node, data_name) else {
            return Ok(());
        };
        if data.len() % comps != 0 {
            return Err(err("vertex element size"));
        }
        let num_elems = data.len() / comps;
        if num_elems == 0 {
            return Ok(());
        }
        if num_elems >= i32::MAX as usize {
            return Err(err("vertex element count"));
        }
        let indices = env.find_ints(node, index_name);
        attrib.exists = true;
        let mut mapping = env
            .find_str(node, b"MappingInformationType")
            .unwrap_or_default();
        attrib.values = data;
        let n = self.num_indices;
        if mapping_is(&mapping, "ByPolygon") {
            let num = indices.as_ref().map_or(num_elems, Vec::len);
            if num == n {
                mapping = b"ByPolygonVertex".to_vec();
            }
        }
        let by_vertex = mapping_is(&mapping, "ByVertex") || mapping_is(&mapping, "ByVertice");
        if let Some(index_data) = indices {
            let index_data: Vec<u32> = index_data
                .into_iter()
                .map(crate::numeric::index_word)
                .collect();
            if mapping_is(&mapping, "ByPolygonVertex") {
                attrib.indices = check_indices(index_data, n, num_elems)?;
            } else if by_vertex {
                let mut new = Vec::with_capacity(n);
                for &ix in &self.position.indices {
                    new.push(if (ix as usize) < index_data.len() {
                        index_data[ix as usize]
                    } else {
                        fix_index(num_elems)?
                    });
                }
                attrib.indices = check_indices(new, n, num_elems)?;
                attrib.unique_per_vertex = true;
            } else if mapping_is(&mapping, "ByPolygon") {
                let mut new = vec![0u32; n];
                for (face_ix, &(begin, count)) in self.faces.iter().enumerate() {
                    let mut index = index_data.get(face_ix).copied().unwrap_or(NO_INDEX);
                    if index as usize >= num_elems {
                        index = fix_index(num_elems)?;
                    }
                    for i in 0..count {
                        new[(begin + i) as usize] = index;
                    }
                }
                attrib.indices = new;
            } else if mapping_is(&mapping, "AllSame") {
                attrib.indices = vec![0; n];
                attrib.unique_per_vertex = true;
            } else {
                *attrib = Attrib::default();
            }
        } else if mapping_is(&mapping, "ByPolygonVertex") {
            let consecutive: Vec<u32> =
                (0..u32::try_from(n).map_err(|_| err("index count overflow"))?).collect();
            attrib.indices = if num_elems >= n {
                consecutive
            } else {
                check_indices(consecutive, n, num_elems)?
            };
        } else if by_vertex {
            attrib.indices = check_indices(self.position.indices.clone(), n, num_elems)?;
            attrib.unique_per_vertex = true;
        } else if mapping_is(&mapping, "ByPolygon") {
            let mut new = vec![0u32; n];
            for (face_ix, &(begin, count)) in self.faces.iter().enumerate() {
                for i in 0..count {
                    new[(begin + i) as usize] =
                        u32::try_from(face_ix).map_err(|_| err("index count overflow"))?;
                }
            }
            attrib.indices = check_indices(new, n, num_elems)?;
        } else if mapping_is(&mapping, "AllSame") {
            attrib.indices = vec![0; n];
            attrib.unique_per_vertex = true;
        } else {
            *attrib = Attrib::default();
        }
        Ok(())
    }

    /// `ufbxi_process_indices`.
    fn process_indices(&mut self, index_data: &mut [u32]) -> Result<(), String> {
        let mut begin = 0usize;
        for (i, slot) in index_data.iter_mut().enumerate() {
            let mut ix = *slot;
            if (crate::numeric::signed_word(ix)) < 0 {
                ix = !ix;
                *slot = ix;
                let num = u32::try_from(i - begin + 1).map_err(|_| err("index count overflow"))?;
                self.faces.push((
                    u32::try_from(begin).map_err(|_| err("index count overflow"))?,
                    num,
                ));
                if num >= 3 {
                    self.num_triangles += u64::from(num - 2);
                    self.max_face_triangles = self.max_face_triangles.max(u64::from(num - 2));
                } else {
                    self.num_bad[num as usize] += 1;
                }
                begin = i + 1;
            }
            if ix as usize >= self.num_vertices {
                return Err(err("vertex index out of range"));
            }
        }
        self.vertex_first_index = vec![NO_INDEX; self.num_vertices];
        for (ix, &vx) in index_data.iter().enumerate() {
            if (vx as usize) < self.num_vertices && self.vertex_first_index[vx as usize] == NO_INDEX
            {
                self.vertex_first_index[vx as usize] =
                    u32::try_from(ix).map_err(|_| err("index count overflow"))?;
            }
        }
        Ok(())
    }

    /// `ufbxi_assign_face_groups` with retained parts.
    fn assign_face_groups(&mut self) -> Result<(), String> {
        let num_faces = self.faces.len();
        let Some(face_group) = self.face_group.as_mut() else {
            return Ok(());
        };
        if num_faces == 0 || face_group.len() != num_faces {
            return Err(err("face groups"));
        }
        let mut ids: Vec<i32> = face_group
            .iter()
            .map(|&x| crate::numeric::signed_word(x))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        self.face_groups = ids.clone();
        let mut parts: Vec<Part> = vec![Part::default(); ids.len()];
        if ids.len() == 1 {
            face_group.iter_mut().for_each(|x| *x = 0);
            parts[0].face_indices =
                (0..u32::try_from(num_faces).map_err(|_| err("index count overflow"))?).collect();
            parts[0].num_bad = self.num_bad;
            parts[0].num_faces = num_faces as u64;
            parts[0].num_triangles = self.num_triangles;
            self.face_group_parts = parts;
            return Ok(());
        }
        for (f, g) in face_group.iter_mut().enumerate() {
            let index = ids
                .binary_search(&(crate::numeric::signed_word(*g)))
                .map_err(|_| err("face group lookup"))?;
            parts[index].add_face(self.faces[f].1);
            *g = u32::try_from(index).map_err(|_| err("index count overflow"))?;
        }
        for (i, p) in parts.iter_mut().enumerate() {
            p.index = u32::try_from(i).map_err(|_| err("index count overflow"))?;
        }
        for (f, &g) in face_group.iter().enumerate() {
            parts[g as usize]
                .face_indices
                .push(u32::try_from(f).map_err(|_| err("index count overflow"))?);
        }
        self.face_group_parts = parts;
        Ok(())
    }

    fn vec3(&self, a: &Attrib, index: usize) -> [f64; 3] {
        let i = a.indices[index] as usize * 3;
        [a.values[i], a.values[i + 1], a.values[i + 2]]
    }

    /// `ufbx_get_weighted_face_normal`.
    fn weighted_face_normal(&self, (begin, count): (u32, u32)) -> [f64; 3] {
        let p = &self.position;
        let b = begin as usize;
        let sub = |a: [f64; 3], c: [f64; 3]| [a[0] - c[0], a[1] - c[1], a[2] - c[2]];
        let cross = |a: [f64; 3], c: [f64; 3]| {
            [
                a[1] * c[2] - a[2] * c[1],
                a[2] * c[0] - a[0] * c[2],
                a[0] * c[1] - a[1] * c[0],
            ]
        };
        match count {
            0..=2 => [0.0; 3],
            3 => {
                let (a, bb, c) = (self.vec3(p, b), self.vec3(p, b + 1), self.vec3(p, b + 2));
                cross(sub(bb, a), sub(c, a))
            }
            4 => {
                let (a, bb, c, d) = (
                    self.vec3(p, b),
                    self.vec3(p, b + 1),
                    self.vec3(p, b + 2),
                    self.vec3(p, b + 3),
                );
                cross(sub(c, a), sub(d, bb))
            }
            _ => {
                let mut r = [0.0; 3];
                let n = count as usize;
                for i in 0..n {
                    let next = if i + 1 < n { i + 1 } else { 0 };
                    let a = self.vec3(p, b + i);
                    let c = self.vec3(p, b + next);
                    r[0] += (a[1] - c[1]) * (a[2] + c[2]);
                    r[1] += (a[2] - c[2]) * (a[0] + c[0]);
                    r[2] += (a[0] - c[0]) * (a[1] + c[1]);
                }
                r
            }
        }
    }

    /// `ufbxi_generate_normals` (topology, smoothing-aware normal mapping and
    /// area-weighted face normals; `assume_smooth = false`).
    fn generate_normals(&mut self, vertex_indices: &[u32]) {
        self.generated_normals = true;
        let n = self.num_indices;
        // ufbx_topo_edge: index, next, prev, twin, face, edge, flags(non-manifold)
        #[derive(Clone, Copy)]
        struct Topo {
            index: u32,
            next: u32,
            prev: u32,
            twin: u32,
            face: u32,
            edge: u32,
        }
        let mut topo = Vec::with_capacity(n);
        for (fi, &(begin, count)) in self.faces.iter().enumerate() {
            for pi in 0..count {
                let ni = (pi + 1) % count;
                let mut va = vertex_indices[(begin + pi) as usize];
                let mut vb = vertex_indices[(begin + ni) as usize];
                if vb < va {
                    std::mem::swap(&mut va, &mut vb);
                }
                topo.push(Topo {
                    index: begin + pi,
                    twin: NO_INDEX,
                    edge: NO_INDEX,
                    prev: va,
                    next: vb,
                    face: u32::try_from(fi).expect("face count bounded by index count"),
                });
            }
        }
        topo.sort_by_key(|t| {
            (
                crate::numeric::signed_word(t.prev),
                crate::numeric::signed_word(t.next),
            )
        });
        if let Some(edges) = &self.edges {
            for (ei, e) in edges.iter().enumerate() {
                let mut va = vertex_indices[e[0] as usize];
                let mut vb = vertex_indices[e[1] as usize];
                if vb < va {
                    std::mem::swap(&mut va, &mut vb);
                }
                let mut ix = topo.partition_point(|a| {
                    if a.prev == va {
                        a.next < vb
                    } else {
                        a.prev < va
                    }
                });
                while ix < n && topo[ix].prev == va && topo[ix].next == vb {
                    topo[ix].edge = u32::try_from(ei).expect("edge count preflighted");
                    ix += 1;
                }
            }
        }
        let mut i0 = 0;
        while i0 < n {
            let mut i1 = i0;
            let (a, b) = (topo[i0].prev, topo[i0].next);
            while i1 + 1 < n && topo[i1 + 1].prev == a && topo[i1 + 1].next == b {
                i1 += 1;
            }
            if i1 == i0 + 1 {
                topo[i0].twin = topo[i1].index;
                topo[i1].twin = topo[i0].index;
            }
            i0 = i1 + 1;
        }
        topo.sort_by_key(|t| crate::numeric::signed_word(t.index));
        for &(begin, count) in &self.faces {
            for i in 0..count {
                let t = &mut topo[(begin + i) as usize];
                t.prev = begin + (i + count - 1) % count;
                t.next = begin + (i + 1) % count;
            }
        }
        let smooth = |index: u32| -> bool {
            let t = &topo[index as usize];
            if let Some(es) = &self.edge_smoothing {
                if t.edge != NO_INDEX && es[t.edge as usize] {
                    return true;
                }
            }
            if let Some(fs) = &self.face_smoothing {
                if fs[t.face as usize] {
                    return true;
                }
                if t.twin != NO_INDEX && fs[topo[t.twin as usize].face as usize] {
                    return true;
                }
            }
            false
        };
        let next_vertex_edge = |index: u32| -> u32 {
            let twin = topo[index as usize].twin;
            if twin == NO_INDEX {
                NO_INDEX
            } else {
                topo[twin as usize].next
            }
        };
        let prev_vertex_edge =
            |index: u32| -> u32 { topo[topo[index as usize].prev as usize].twin };
        let mut normal_indices = vec![NO_INDEX; n];
        let mut next_index = 0u32;
        for vi in 0..self.num_vertices {
            let original_start = self.vertex_first_index[vi];
            if original_start == NO_INDEX {
                continue;
            }
            let mut start = original_start;
            let mut cur = start;
            loop {
                let prev = next_vertex_edge(cur);
                if !smooth(cur) {
                    start = cur;
                }
                if prev == NO_INDEX {
                    start = cur;
                    break;
                }
                if prev == original_start {
                    break;
                }
                cur = prev;
            }
            normal_indices[start as usize] = next_index;
            next_index = next_index.wrapping_add(1);
            let mut next = start;
            loop {
                next = prev_vertex_edge(next);
                if next == NO_INDEX || next == start {
                    break;
                }
                if !smooth(next) {
                    next_index = next_index.wrapping_add(1);
                }
                normal_indices[next as usize] = next_index.wrapping_sub(1);
            }
        }
        for x in normal_indices.iter_mut() {
            if *x == NO_INDEX {
                *x = next_index;
                next_index = next_index.wrapping_add(1);
            }
        }
        let num_normals = next_index as usize;
        let mut normals = vec![0.0f64; num_normals * 3];
        for &face in &self.faces {
            let fnormal = self.weighted_face_normal(face);
            for ix in 0..face.1 {
                let index = normal_indices[(face.0 + ix) as usize] as usize;
                let nrm = &mut normals[index * 3..index * 3 + 3];
                nrm[0] += fnormal[0];
                nrm[1] += fnormal[1];
                nrm[2] += fnormal[2];
            }
        }
        for nrm in normals.as_chunks_mut::<3>().0 {
            let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
            if len > 0.0 {
                nrm[0] /= len;
                nrm[1] /= len;
                nrm[2] /= len;
            }
        }
        self.normal = Attrib {
            exists: true,
            values: normals,
            indices: normal_indices,
            unique_per_vertex: num_normals == self.num_vertices,
        };
    }
}

/// `ufbxi_mirror_vec3_list` over flat xyz values.
fn mirror(values: &mut [f64], axis: i64) {
    if axis == 0 {
        return;
    }
    let k = usize::try_from(axis - 1).expect("validated mirror axis");
    for v in values.as_chunks_mut::<3>().0 {
        v[k] = -v[k];
    }
}

/// `ufbx_transform_position` over flat xyz values.
fn transform(values: &mut [f64], m: &Mat) {
    for v in values.as_chunks_mut::<3>().0 {
        let (x, y, z) = (v[0], v[1], v[2]);
        v[0] = m[0] * x + m[3] * y + m[6] * z + m[9];
        v[1] = m[1] * x + m[4] * y + m[7] * z + m[10];
        v[2] = m[2] * x + m[5] * y + m[8] * z + m[11];
    }
}

/// `ufbxi_normalize3` over flat xyz values.
fn normalize(values: &mut [f64]) {
    for v in values.as_chunks_mut::<3>().0 {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len > EPSILON {
            let r = 1.0 / len;
            v[0] *= r;
            v[1] *= r;
            v[2] *= r;
        } else {
            v.fill(0.0);
        }
    }
}

/// `ufbx_matrix_for_normals`.
fn matrix_for_normals(m: &Mat) -> Mat {
    let g = |r: usize, c: usize| m[c * 3 + r];
    let det = sn::matrix_determinant(m);
    let sign = if det >= 0.0 { 1.0 } else { -1.0 };
    let mut r = [0.0; 12];
    let mut set = |row: usize, c: usize, v: f64| r[c * 3 + row] = v;
    set(0, 0, (-g(1, 2) * g(2, 1) + g(1, 1) * g(2, 2)) * sign);
    set(0, 1, (g(1, 2) * g(2, 0) - g(1, 0) * g(2, 2)) * sign);
    set(0, 2, (-g(1, 1) * g(2, 0) + g(1, 0) * g(2, 1)) * sign);
    set(1, 0, (g(0, 2) * g(2, 1) - g(0, 1) * g(2, 2)) * sign);
    set(1, 1, (-g(0, 2) * g(2, 0) + g(0, 0) * g(2, 2)) * sign);
    set(1, 2, (g(0, 1) * g(2, 0) - g(0, 0) * g(2, 1)) * sign);
    set(2, 0, (-g(0, 2) * g(1, 1) + g(0, 1) * g(1, 2)) * sign);
    set(2, 1, (g(0, 2) * g(1, 0) - g(0, 0) * g(1, 2)) * sign);
    set(2, 2, (-g(0, 1) * g(1, 0) + g(0, 0) * g(1, 1)) * sign);
    r
}

/// Reverse each face's indices after its first one (`ufbxi_flip_attrib_winding`).
fn flip(indices: &mut [u32], faces: &[(u32, u32)]) {
    if indices.is_empty() {
        return;
    }
    for &(begin, count) in faces {
        if count == 0 {
            continue;
        }
        indices[(begin + 1) as usize..(begin + count) as usize].reverse();
    }
}

fn read_mesh(env: &MeshEnv<'_, '_>, node: usize) -> Result<Mesh, String> {
    let mut m = Mesh::default();
    let Some(node_vertices) = env.child(node, b"Vertices") else {
        return Ok(m);
    };
    let node_indices = env.child(node, b"PolygonVertexIndex");
    let vertices = env
        .reals(node_vertices)
        .ok_or_else(|| err("Vertices array"))?;
    let indices = match node_indices {
        Some(n) => Some(env.ints(n).ok_or_else(|| err("PolygonVertexIndex array"))?),
        None => None,
    };
    let edge_indices = env.find_ints(node, b"Edges");
    if vertices.len() % 3 != 0 {
        return Err(err("vertex count"));
    }
    m.num_vertices = vertices.len() / 3;
    let mut index_data: Vec<u32> = indices
        .unwrap_or_default()
        .into_iter()
        .map(crate::numeric::index_word)
        .collect();
    m.num_indices = index_data.len();
    if let Some(last) = index_data.last_mut() {
        if (crate::numeric::signed_word(*last)) >= 0 {
            *last = !*last;
        }
    }
    if let Some(edge_data) = edge_indices {
        let n = m.num_indices;
        let mut edges = Vec::with_capacity(edge_data.len());
        for e in edge_data {
            let a = crate::numeric::index_word(e);
            if a as usize >= n {
                continue;
            }
            let mut ix = a as usize;
            if (crate::numeric::signed_word(index_data[ix])) < 0 {
                while ix > 0 && (crate::numeric::signed_word(index_data[ix - 1])) >= 0 {
                    ix -= 1;
                }
            } else {
                ix += 1;
            }
            if ix >= n {
                return Err(err("edge index"));
            }
            edges.push([
                a,
                u32::try_from(ix).map_err(|_| err("index count overflow"))?,
            ]);
        }
        m.edges = Some(edges);
    }
    m.process_indices(&mut index_data)?;
    m.position = Attrib {
        exists: true,
        values: vertices,
        indices: index_data,
        unique_per_vertex: true,
    };
    let num_faces = m.faces.len();
    let num_edges = m.edges.as_ref().map_or(0, Vec::len);

    let mut bitangents: Vec<(u32, usize)> = Vec::new();
    let mut tangents: Vec<(u32, usize)> = Vec::new();
    let (mut stale_bi, mut stale_tan, mut stale_uv, mut stale_color) = (0u32, 0u32, 0u32, 0u32);
    let children = env.children[node].clone();
    for &n in &children {
        let name = env.doc.nodes[n].name;
        if name.first() != Some(&b'L') {
            continue;
        }
        match name {
            b"LayerElementNormal" => {
                if m.normal.exists {
                    continue;
                }
                let mut a = std::mem::take(&mut m.normal);
                m.read_vertex_element(env, n, &mut a, b"Normals", b"NormalsIndex", 3)?;
                m.normal = a;
            }
            b"LayerElementBinormal" | b"LayerElementTangent" => {
                let bi = name == b"LayerElementBinormal";
                let stale = if bi { &mut stale_bi } else { &mut stale_tan };
                let index = env.val_i32(n).map_or(*stale, crate::numeric::index_word);
                let mut a = Attrib::default();
                if bi {
                    m.read_vertex_element(env, n, &mut a, b"Binormals", b"BinormalsIndex", 3)?;
                } else {
                    m.read_vertex_element(env, n, &mut a, b"Tangents", b"TangentsIndex", 3)?;
                }
                if a.exists {
                    *stale = 0;
                    m.tangent_layers.push(a);
                    let slot = (index, m.tangent_layers.len() - 1);
                    if bi {
                        bitangents.push(slot);
                    } else {
                        tangents.push(slot);
                    }
                } else {
                    *stale = index;
                }
            }
            b"LayerElementUV" => {
                let index = env.val_i32(n).map_or(stale_uv, crate::numeric::index_word);
                let mut set = UvSet {
                    index,
                    name: env.find_str(n, b"Name").unwrap_or_default(),
                    ..UvSet::default()
                };
                m.read_vertex_element(env, n, &mut set.uv, b"UV", b"UVIndex", 2)?;
                if set.uv.exists {
                    stale_uv = 0;
                    m.uv_sets.push(set);
                } else {
                    stale_uv = index;
                }
            }
            b"LayerElementColor" => {
                let index = env
                    .val_i32(n)
                    .map_or(stale_color, crate::numeric::index_word);
                let mut set = ColorSet {
                    index,
                    name: env.find_str(n, b"Name").unwrap_or_default(),
                    ..ColorSet::default()
                };
                m.read_vertex_element(env, n, &mut set.color, b"Colors", b"ColorIndex", 4)?;
                if set.color.exists {
                    stale_color = 0;
                    m.color_sets.push(set);
                } else {
                    stale_color = index;
                }
            }
            b"LayerElementVertexCrease" => {
                let mut a = std::mem::take(&mut m.crease);
                m.read_vertex_element(env, n, &mut a, b"VertexCrease", b"VertexCreaseIndex", 1)?;
                m.crease = a;
            }
            b"LayerElementEdgeCrease" | b"LayerElementSmoothing" | b"LayerElementVisibility" => {
                let mapping = env
                    .find_str(n, b"MappingInformationType")
                    .unwrap_or_default();
                let by_edge = mapping_is(&mapping, "ByEdge");
                let by_polygon = mapping_is(&mapping, "ByPolygon");
                match name {
                    b"LayerElementEdgeCrease" if by_edge => {
                        if m.edge_crease.as_ref().is_some_and(|v| !v.is_empty()) {
                            continue;
                        }
                        if let Some(v) = truncated(env.find_reals(n, b"EdgeCrease"), num_edges) {
                            m.edge_crease = Some(v);
                        }
                    }
                    b"LayerElementSmoothing" if by_edge => {
                        if m.edge_smoothing.as_ref().is_some_and(|v| !v.is_empty()) {
                            continue;
                        }
                        if let Some(v) = truncated(env.find_bools(n, b"Smoothing"), num_edges) {
                            m.edge_smoothing = Some(v);
                        }
                    }
                    b"LayerElementSmoothing" if by_polygon => {
                        if m.face_smoothing.as_ref().is_some_and(|v| !v.is_empty()) {
                            continue;
                        }
                        if let Some(v) = truncated(env.find_bools(n, b"Smoothing"), num_faces) {
                            m.face_smoothing = Some(v);
                        }
                    }
                    b"LayerElementVisibility" if by_edge => {
                        if m.edge_visibility.as_ref().is_some_and(|v| !v.is_empty()) {
                            continue;
                        }
                        if let Some(v) = truncated(env.find_bools(n, b"Visibility"), num_edges) {
                            m.edge_visibility = Some(v);
                        }
                    }
                    _ => {}
                }
            }
            b"LayerElementMaterial" => {
                if m.face_material.as_ref().is_some_and(|v| !v.is_empty()) {
                    continue;
                }
                let mapping = env
                    .find_str(n, b"MappingInformationType")
                    .unwrap_or_default();
                if mapping_is(&mapping, "ByPolygon") {
                    let arr = env
                        .find_ints(n, b"Materials")
                        .map(|v| v.into_iter().map(crate::numeric::index_word).collect());
                    if let Some(v) = truncated(arr, num_faces) {
                        m.face_material = Some(v);
                    }
                } else if mapping_is(&mapping, "AllSame") {
                    let arr = env.find_ints(n, b"Materials");
                    let material = match arr.as_deref() {
                        Some([first, ..]) => crate::numeric::index_word(*first),
                        _ => return Err(err("AllSame materials")),
                    };
                    m.face_material = Some(vec![material; num_faces]);
                }
            }
            b"LayerElementPolygonGroup" | b"LayerElementHole" => {
                if m.face_group.as_ref().is_some_and(|v| !v.is_empty()) {
                    continue;
                }
                let mapping = env
                    .find_str(n, b"MappingInformationType")
                    .ok_or_else(|| err("polygon group mapping"))?;
                if mapping_is(&mapping, "ByPolygon") {
                    if name == b"LayerElementPolygonGroup" {
                        let arr = env
                            .find_ints(n, b"PolygonGroup")
                            .map(|v| v.into_iter().map(crate::numeric::index_word).collect());
                        if let Some(v) = truncated(arr, num_faces) {
                            m.face_group = Some(v);
                        }
                    } else if let Some(v) = truncated(env.find_bools(n, b"Hole"), num_faces) {
                        m.face_hole = Some(v);
                    }
                }
            }
            _ => {}
        }
    }
    if !m.face_material.as_ref().is_some_and(|v| !v.is_empty()) {
        m.face_material = Some(vec![0; num_faces]);
    }

    // Connect bitangents/tangents to UV sets through `Layer` declarations.
    for &l in &children {
        if env.doc.nodes[l].name != b"Layer" {
            continue;
        }
        let (mut uv_set, mut bi, mut tan) = (None, None, None);
        for &c in &env.children[l] {
            if env.doc.nodes[c].name != b"LayerElement" {
                continue;
            }
            let Some(index) = env
                .find_i32(c, b"TypedIndex")
                .map(crate::numeric::index_word)
            else {
                continue;
            };
            let Some(ty) = env.find_str(c, b"Type") else {
                continue;
            };
            match ty.as_slice() {
                b"LayerElementUV" => {
                    uv_set = m.uv_sets.iter().position(|s| s.index == index);
                }
                b"LayerElementBinormal" => {
                    bi = bitangents.iter().find(|t| t.0 == index).map(|t| t.1);
                }
                b"LayerElementTangent" => {
                    tan = tangents.iter().find(|t| t.0 == index).map(|t| t.1);
                }
                _ => {}
            }
        }
        if let Some(u) = uv_set {
            if bi.is_some() {
                m.uv_sets[u].bitangent = bi;
            }
            if tan.is_some() {
                m.uv_sets[u].tangent = tan;
            }
        }
    }

    m.skinned_is_local = true;
    if m.face_group.as_ref().is_some_and(|v| !v.is_empty()) && m.face_groups.is_empty() {
        m.assign_face_groups()?;
    }
    m.uv_sets.sort_by_key(|s| s.index);
    m.color_sets.sort_by_key(|s| s.index);

    if let Some(v) = env.find_i32(node, b"PreviewDivisionLevels") {
        m.subdivision_preview_levels = u64::from(crate::numeric::index_word(v));
    }
    if let Some(v) = env.find_i32(node, b"RenderDivisionLevels") {
        m.subdivision_render_levels = u64::from(crate::numeric::index_word(v));
    }
    if let Some(v) = env.find_i32(node, b"Smoothness") {
        if (0..=3).contains(&v) {
            m.subdivision_display_mode = i64::from(v);
        }
    }
    if let Some(v) = env.find_i32(node, b"BoundaryRule") {
        if (0..=1).contains(&v) {
            m.subdivision_boundary = i64::from(v) + 1;
        }
    }
    Ok(m)
}

fn attrib_obj(a: &Attrib, comps: usize) -> V {
    let values = match comps {
        1 => V::F64s(a.values.clone()),
        2 => V::Vec2s(a.values.as_chunks::<2>().0.to_vec()),
        3 => V::Vec3s(a.values.as_chunks::<3>().0.to_vec()),
        _ => V::Vec4s(a.values.as_chunks::<4>().0.to_vec()),
    };
    V::Obj(
        Obj::new()
            .with("exists", V::Bool(a.exists))
            .with("values", values)
            .with("indices", V::U32s(a.indices.clone()))
            .with("value_reals", V::U(comps as u64))
            .with("unique_per_vertex", V::Bool(a.unique_per_vertex)),
    )
}

fn part_obj(p: &Part) -> V {
    V::Obj(
        Obj::new()
            .with("index", V::U(u64::from(p.index)))
            .with("num_faces", V::U(p.num_faces))
            .with("num_triangles", V::U(p.num_triangles))
            .with("num_empty_faces", V::U(p.num_bad[0]))
            .with("num_point_faces", V::U(p.num_bad[1]))
            .with("num_line_faces", V::U(p.num_bad[2]))
            .with("face_indices", V::U32s(p.face_indices.clone())),
    )
}

fn refs(v: &[usize]) -> V {
    V::List(v.iter().map(|&k| V::Ref(Some(k))).collect())
}

/// Project one `ufbx_mesh` (fields after `element`) into `obj`.
/// Returns the mesh's `max_face_triangles` for the scene metadata.
pub fn project_mesh(
    env: &MeshEnv<'_, '_>,
    node: usize,
    links: &MeshLinks,
    obj: &mut Obj,
) -> Result<u64, String> {
    let m = convert_mesh(env, node, links)?;
    emit_mesh(&m, links, obj);
    Ok(m.max_face_triangles)
}

/// Convert one `ufbx_mesh` under the scene options (normals, parts, space
/// conversion, geometry transform) without building a canonical record.
pub(crate) fn convert_mesh(
    env: &MeshEnv<'_, '_>,
    node: usize,
    links: &MeshLinks,
) -> Result<Mesh, String> {
    // Source counts bound every derived corner/face/edge index, including
    // the normal generator's infallible conversions. Check before decoding.
    for &child in &env.children[node] {
        if crate::canon_dom::data_len(env.doc, child)? > usize::try_from(i32::MAX).unwrap() {
            return Err(err("source array count overflow"));
        }
    }
    if links.materials.len() > usize::try_from(i32::MAX).unwrap() {
        return Err(err("material count overflow"));
    }

    let mut m = read_mesh(env, node)?;

    // `ufbxi_finalize_scene`.
    if !m.normal.exists {
        let vertex_indices = m.position.indices.clone();
        m.generate_normals(&vertex_indices);
    }
    let num_faces = m.faces.len();
    let num_materials = links.materials.len();
    let num_parts = num_materials.max(1);
    m.material_parts = vec![Part::default(); num_parts];
    if num_materials <= 1 {
        let p = &mut m.material_parts[0];
        p.num_faces = num_faces as u64;
        p.num_triangles = m.num_triangles;
        p.num_bad = m.num_bad;
        p.face_indices =
            (0..u32::try_from(num_faces).map_err(|_| err("index count overflow"))?).collect();
        m.material_part_usage_order = vec![0];
        m.face_material = (num_materials == 1).then(|| vec![0; num_faces]);
    } else {
        let mut fm = m.face_material.take().unwrap_or_default();
        for (i, face) in m.faces.iter().enumerate() {
            let mut mat = 0;
            if let Some(x) = fm.get_mut(i) {
                if *x as usize >= num_materials {
                    *x = 0;
                }
                mat = *x as usize;
            }
            m.material_parts[mat].add_face(face.1);
        }
        for (i, p) in m.material_parts.iter_mut().enumerate() {
            p.index = u32::try_from(i).map_err(|_| err("index count overflow"))?;
            p.face_indices
                .reserve(usize::try_from(p.num_faces).map_err(|_| err("face count overflow"))?);
        }
        for (i, &mat) in fm.iter().enumerate().take(num_faces) {
            if (mat as usize) < num_parts {
                m.material_parts[mat as usize]
                    .face_indices
                    .push(u32::try_from(i).map_err(|_| err("index count overflow"))?);
            }
        }
        let parts = &m.material_parts;
        let mut order: Vec<u32> =
            (0..u32::try_from(num_parts).map_err(|_| err("index count overflow"))?).collect();
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&parts[a as usize], &parts[b as usize]);
            let (ca, cb) = (pa.face_indices.len(), pb.face_indices.len());
            if ca == 0 || cb == 0 {
                if ca == cb {
                    a.cmp(&b)
                } else {
                    cb.cmp(&ca)
                }
            } else {
                pa.face_indices[0].cmp(&pb.face_indices[0])
            }
        });
        m.material_part_usage_order = order;
        m.face_material = Some(fm);
    }
    if !m.position.exists {
        if m.num_indices != 0 {
            return Err(err("missing vertex positions"));
        }
        m.position.exists = true;
        m.position.unique_per_vertex = true;
    }

    // `ufbxi_modify_geometry`.
    if env.geometry_scale != 1.0 {
        let s = env.geometry_scale;
        for x in m.position.values.iter_mut() {
            *x *= s;
        }
    }
    if env.mirror_axis != 0 {
        mirror(&mut m.position.values, env.mirror_axis);
        mirror(&mut m.normal.values, env.mirror_axis);
        for k in 0..m.uv_sets.len() {
            for layer in [m.uv_sets[k].tangent, m.uv_sets[k].bitangent]
                .into_iter()
                .flatten()
            {
                mirror(&mut m.tangent_layers[layer].values, env.mirror_axis);
            }
        }
        m.reversed_winding = true;
        let faces = std::mem::take(&mut m.faces);
        flip(&mut m.position.indices, &faces);
        flip(&mut m.normal.indices, &faces);
        flip(&mut m.crease.indices, &faces);
        let mut flipped = vec![false; m.tangent_layers.len()];
        for set in m.uv_sets.iter_mut() {
            flip(&mut set.uv.indices, &faces);
            for layer in [set.tangent, set.bitangent].into_iter().flatten() {
                if !flipped[layer] {
                    flipped[layer] = true;
                    flip(&mut m.tangent_layers[layer].indices, &faces);
                }
            }
        }
        for set in m.color_sets.iter_mut() {
            flip(&mut set.color.indices, &faces);
        }
        m.vertex_first_index.fill(NO_INDEX);
        for (ix, &vx) in m.position.indices.iter().enumerate() {
            if (vx as usize) < m.num_vertices && m.vertex_first_index[vx as usize] == NO_INDEX {
                m.vertex_first_index[vx as usize] =
                    u32::try_from(ix).map_err(|_| err("index count overflow"))?;
            }
        }
        if let Some(edges) = m.edges.as_mut() {
            if !edges.is_empty() {
                let mut map = vec![NO_INDEX; m.num_indices];
                for &(begin, count) in &faces {
                    if count == 0 {
                        continue;
                    }
                    let c = count - 1;
                    map[begin as usize] = begin;
                    for i in 0..c {
                        map[(begin + 1 + i) as usize] = begin + c - i;
                    }
                }
                for e in edges.iter_mut() {
                    let (a, b) = (map[e[0] as usize], map[e[1] as usize]);
                    *e = [b, a];
                }
            }
        }
        m.faces = faces;
    }
    if let Some(g) = &links.geometry_to_node {
        let mut tangent_matrix = *g;
        tangent_matrix[9] = 0.0;
        tangent_matrix[10] = 0.0;
        tangent_matrix[11] = 0.0;
        let normal_matrix = matrix_for_normals(g);
        transform(&mut m.position.values, g);
        transform(&mut m.normal.values, &normal_matrix);
        normalize(&mut m.normal.values);
        for k in 0..m.uv_sets.len() {
            for layer in [m.uv_sets[k].tangent, m.uv_sets[k].bitangent]
                .into_iter()
                .flatten()
            {
                transform(&mut m.tangent_layers[layer].values, &tangent_matrix);
                normalize(&mut m.tangent_layers[layer].values);
            }
        }
    }
    // `ufbxi_postprocess_scene` (normalize_normals).
    normalize(&mut m.normal.values);
    Ok(m)
}

/// Write the canonical `ufbx_mesh` fields of a converted mesh into `obj`.
fn emit_mesh(m: &Mesh, links: &MeshLinks, obj: &mut Obj) {
    let num_faces = m.faces.len();
    let empty = Attrib::default();
    let layer = |l: Option<usize>| l.map_or(&empty, |k| &m.tangent_layers[k]);
    let face_list = V::List(
        m.faces
            .iter()
            .map(|&(b, n)| {
                V::Obj(
                    Obj::new()
                        .with("index_begin", V::U(u64::from(b)))
                        .with("num_indices", V::U(u64::from(n))),
                )
            })
            .collect(),
    );
    let edges = m.edges.clone().unwrap_or_default();
    let first_uv = m.uv_sets.first();
    let first_color = m.color_sets.first();
    let (uv, tangent, bitangent) = match first_uv {
        Some(u) => (&u.uv, layer(u.tangent), layer(u.bitangent)),
        None => (&empty, &empty, &empty),
    };
    let color = first_color.map_or(&empty, |c| &c.color);
    obj.set("num_vertices", V::U(m.num_vertices as u64))
        .set("num_indices", V::U(m.num_indices as u64))
        .set("num_faces", V::U(num_faces as u64))
        .set("num_triangles", V::U(m.num_triangles))
        .set("num_edges", V::U(edges.len() as u64))
        .set("max_face_triangles", V::U(m.max_face_triangles))
        .set("num_empty_faces", V::U(m.num_bad[0]))
        .set("num_point_faces", V::U(m.num_bad[1]))
        .set("num_line_faces", V::U(m.num_bad[2]))
        .set("faces", face_list)
        .set(
            "face_smoothing",
            V::Bools(m.face_smoothing.clone().unwrap_or_default()),
        )
        .set(
            "face_material",
            V::U32s(m.face_material.clone().unwrap_or_default()),
        )
        .set(
            "face_group",
            V::U32s(m.face_group.clone().unwrap_or_default()),
        )
        .set(
            "face_hole",
            V::Bools(m.face_hole.clone().unwrap_or_default()),
        )
        .set(
            "edges",
            V::List(
                edges
                    .iter()
                    .map(|e| V::Obj(Obj::new().with("indices", V::U32s(e.to_vec()))))
                    .collect(),
            ),
        )
        .set(
            "edge_smoothing",
            V::Bools(m.edge_smoothing.clone().unwrap_or_default()),
        )
        .set(
            "edge_crease",
            V::F64s(m.edge_crease.clone().unwrap_or_default()),
        )
        .set(
            "edge_visibility",
            V::Bools(m.edge_visibility.clone().unwrap_or_default()),
        )
        .set("vertex_indices", V::U32s(m.position.indices.clone()))
        .set(
            "vertices",
            V::Vec3s(m.position.values.as_chunks::<3>().0.to_vec()),
        )
        .set("vertex_first_index", V::U32s(m.vertex_first_index.clone()))
        .set("vertex_position", attrib_obj(&m.position, 3))
        .set("vertex_normal", attrib_obj(&m.normal, 3))
        .set("vertex_uv", attrib_obj(uv, 2))
        .set("vertex_tangent", attrib_obj(tangent, 3))
        .set("vertex_bitangent", attrib_obj(bitangent, 3))
        .set("vertex_color", attrib_obj(color, 4))
        .set("vertex_crease", attrib_obj(&m.crease, 1))
        .set(
            "uv_sets",
            V::List(
                m.uv_sets
                    .iter()
                    .map(|u| {
                        V::Obj(
                            Obj::new()
                                .with("name", s(&u.name))
                                .with("index", V::U(u64::from(u.index)))
                                .with("vertex_uv", attrib_obj(&u.uv, 2))
                                .with("vertex_tangent", attrib_obj(layer(u.tangent), 3))
                                .with("vertex_bitangent", attrib_obj(layer(u.bitangent), 3)),
                        )
                    })
                    .collect(),
            ),
        )
        .set(
            "color_sets",
            V::List(
                m.color_sets
                    .iter()
                    .map(|c| {
                        V::Obj(
                            Obj::new()
                                .with("name", s(&c.name))
                                .with("index", V::U(u64::from(c.index)))
                                .with("vertex_color", attrib_obj(&c.color, 4)),
                        )
                    })
                    .collect(),
            ),
        )
        .set("materials", refs(&links.materials))
        .set(
            "face_groups",
            V::List(
                m.face_groups
                    .iter()
                    .map(|&id| {
                        V::Obj(
                            Obj::new()
                                .with("id", V::I(i64::from(id)))
                                .with("name", s(b"")),
                        )
                    })
                    .collect(),
            ),
        )
        .set(
            "material_parts",
            V::List(m.material_parts.iter().map(part_obj).collect()),
        )
        .set(
            "face_group_parts",
            V::List(m.face_group_parts.iter().map(part_obj).collect()),
        )
        .set(
            "material_part_usage_order",
            V::U32s(m.material_part_usage_order.clone()),
        )
        .set("skinned_is_local", V::Bool(m.skinned_is_local))
        .set("skinned_position", attrib_obj(&m.position, 3))
        .set("skinned_normal", attrib_obj(&m.normal, 3))
        .set("skin_deformers", refs(&links.skin_deformers))
        .set("blend_deformers", refs(&links.blend_deformers))
        .set("cache_deformers", refs(&links.cache_deformers))
        .set("all_deformers", refs(&links.all_deformers))
        .set(
            "subdivision_preview_levels",
            V::U(m.subdivision_preview_levels),
        )
        .set(
            "subdivision_render_levels",
            V::U(m.subdivision_render_levels),
        )
        .set("subdivision_display_mode", V::I(m.subdivision_display_mode))
        .set("subdivision_boundary", V::I(m.subdivision_boundary))
        .set("reversed_winding", V::Bool(m.reversed_winding))
        .set("generated_normals", V::Bool(m.generated_normals));
}

impl Mesh {
    /// Face `i` as (first corner, corner count).
    pub(crate) fn face(&self, i: usize) -> Option<(u32, u32)> {
        self.faces.get(i).copied()
    }
    /// `ufbx_mesh.reversed_winding`.
    pub(crate) fn reversed_winding(&self) -> bool {
        self.reversed_winding
    }
}

fn public_attrib<const N: usize>(a: Attrib) -> Option<crate::api::VertexAttribute<N>> {
    a.exists
        .then(|| crate::api::VertexAttribute::from_flat(a.values, a.indices))
}

/// Move a converted mesh into the public representation. `materials` are
/// public material indices in ufbx order (`ufbx_mesh.materials`).
pub(crate) fn public_mesh(
    m: Mesh,
    base: crate::api::ElementInfo,
    materials: Vec<usize>,
    instances: Vec<usize>,
    triangles: Vec<u32>,
) -> crate::api::Mesh {
    use crate::api;
    let mut tangents = m.tangent_layers.into_iter().map(Some).collect::<Vec<_>>();
    let mut take = |k: Option<usize>| {
        k.and_then(|k| tangents.get_mut(k).and_then(Option::take))
            .and_then(public_attrib::<3>)
    };
    let uv_sets = m
        .uv_sets
        .into_iter()
        .map(|u| api::UvSet {
            name: String::from_utf8_lossy(&u.name).into_owned(),
            uv: public_attrib::<2>(u.uv).unwrap_or_default(),
            tangent: take(u.tangent),
            bitangent: take(u.bitangent),
        })
        .collect();
    let color_sets = m
        .color_sets
        .into_iter()
        .map(|c| api::ColorSet {
            name: String::from_utf8_lossy(&c.name).into_owned(),
            color: public_attrib::<4>(c.color).unwrap_or_default(),
        })
        .collect();
    let parts = m
        .material_parts
        .into_iter()
        .map(|p| api::MeshPart {
            index: p.index,
            num_triangles: p.num_triangles,
            face_indices: p.face_indices,
        })
        .collect();
    api::Mesh {
        base,
        num_vertices: m.num_vertices,
        faces: m
            .faces
            .into_iter()
            .map(|(begin, count)| api::Face { begin, count })
            .collect(),
        positions: public_attrib::<3>(m.position).unwrap_or_default(),
        normals: public_attrib::<3>(m.normal),
        uv_sets,
        color_sets,
        face_material: m.face_material.unwrap_or_default(),
        materials,
        parts,
        triangles,
        instances,
        reversed_winding: m.reversed_winding,
        generated_normals: m.generated_normals,
        skin_deformers: Vec::new(),
        blend_deformers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_normals_split_hard_edges_and_flip_keeps_first_corner() {
        // Two triangles sharing edge 1-2 without smoothing data: ufbx treats
        // every edge as hard, so each corner gets its own normal.
        let mut m = Mesh {
            num_vertices: 4,
            num_indices: 6,
            faces: vec![(0, 3), (3, 3)],
            position: Attrib {
                exists: true,
                values: vec![0., 0., 0., 2., 0., 0., 0., 2., 0., 2., 2., 0.],
                indices: vec![0, 1, 2, 2, 1, 3],
                unique_per_vertex: true,
            },
            vertex_first_index: vec![0, 1, 2, 5],
            ..Mesh::default()
        };
        let vi = m.position.indices.clone();
        m.generate_normals(&vi);
        assert_eq!(m.normal.values.len(), 18);
        assert!(m.normal.values.chunks(3).all(|n| n == [0.0, 0.0, 1.0]));
        assert!(!m.normal.unique_per_vertex);
        // Face smoothing merges the shared corners: 4 normals for 4 vertices.
        m.face_smoothing = Some(vec![true, true]);
        m.generate_normals(&vi);
        assert_eq!(m.normal.values.len(), 12);
        assert!(m.normal.unique_per_vertex);
        let mut ix = m.position.indices.clone();
        flip(&mut ix, &m.faces);
        assert_eq!(ix, [0, 2, 1, 2, 3, 1]);
    }
}
