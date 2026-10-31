//! Cross-check of the public [`fbx_rs::Scene`] against the canonical
//! projection that the corpus comparison proves equal to ufbx: every exposed
//! value must match its canonical record bit for bit.

use fbx_rs::canon::{Obj, V};
use fbx_rs::canon_scene::{CanonScene, DumpLimits, ElementType as T};
use fbx_rs::{MaterialMap, Matrix, Scene, VertexAttribute};

struct Check {
    values: u64,
}

impl Check {
    fn f(&mut self, what: &str, a: &[f64], b: &[f64]) -> Result<(), String> {
        self.values += a.len() as u64;
        if a.len() != b.len() || a.iter().zip(b).any(|(x, y)| x.to_bits() != y.to_bits()) {
            return Err(format!("{what}: public {a:?} != canonical {b:?}"));
        }
        Ok(())
    }
    fn u(&mut self, what: &str, a: &[u32], b: &[u32]) -> Result<(), String> {
        self.values += a.len() as u64;
        if a != b {
            return Err(format!("{what}: index arrays differ"));
        }
        Ok(())
    }
    fn eq<X: PartialEq + std::fmt::Debug>(&mut self, what: &str, a: X, b: X) -> Result<(), String> {
        self.values += 1;
        if a != b {
            return Err(format!("{what}: public {a:?} != canonical {b:?}"));
        }
        Ok(())
    }
}

fn get<'a>(o: &'a Obj, k: &str) -> Option<&'a V> {
    o.get(k)
}

fn floats(v: Option<&V>) -> Vec<f64> {
    match v {
        Some(V::Vec(x) | V::F64s(x)) => x.clone(),
        Some(V::Vec2s(x)) => x.iter().flatten().copied().collect(),
        Some(V::Vec3s(x)) => x.iter().flatten().copied().collect(),
        Some(V::Vec4s(x)) => x.iter().flatten().copied().collect(),
        Some(V::F(x)) => vec![*x],
        _ => Vec::new(),
    }
}

fn u32s(v: Option<&V>) -> Vec<u32> {
    match v {
        Some(V::U32s(x)) => x.clone(),
        _ => Vec::new(),
    }
}

fn attrib<const N: usize>(
    c: &mut Check,
    what: &str,
    public: Option<&VertexAttribute<N>>,
    record: Option<&V>,
) -> Result<(), String> {
    let Some(V::Obj(r)) = record else {
        return Err(format!("{what}: no canonical record"));
    };
    let exists = matches!(get(r, "exists"), Some(V::Bool(true)));
    c.eq(&format!("{what}.exists"), public.is_some(), exists)?;
    if let Some(p) = public {
        let flat: Vec<f64> = p.values().iter().flatten().copied().collect();
        c.f(&format!("{what}.values"), &flat, &floats(get(r, "values")))?;
        c.u(
            &format!("{what}.indices"),
            p.indices(),
            &u32s(get(r, "indices")),
        )?;
    }
    Ok(())
}

fn list(v: Option<&V>) -> &[V] {
    match v {
        Some(V::Obj(o)) => list(o.get("data")),
        Some(V::List(v)) => v,
        _ => &[],
    }
}
fn bytes(v: Option<&V>) -> &[u8] {
    match v {
        Some(V::Bytes(b)) => b,
        _ => &[],
    }
}
fn integer(v: Option<&V>) -> i64 {
    match v {
        Some(V::I(v)) => *v,
        Some(V::U(v)) => i64::try_from(*v).unwrap(),
        _ => 0,
    }
}
fn texture_ref(v: Option<&V>, slots: &[Option<usize>]) -> Option<usize> {
    match v {
        Some(V::Ref(Some(i))) => slots[*i],
        _ => None,
    }
}
fn map(
    c: &mut Check,
    what: &str,
    m: &MaterialMap,
    r: &Obj,
    slots: &[Option<usize>],
) -> Result<(), String> {
    c.eq(
        "map texture",
        m.texture,
        texture_ref(r.get("texture"), slots),
    )?;
    c.eq("map integer", m.value_int, integer(r.get("value_int")))?;
    c.eq(
        "map components",
        m.value_components,
        u64::try_from(integer(r.get("value_components"))).unwrap(),
    )?;
    c.eq(
        "map texture enabled",
        m.texture_enabled,
        matches!(r.get("texture_enabled"), Some(V::Bool(true))),
    )?;
    c.eq(
        "map feature disabled",
        m.feature_disabled,
        matches!(r.get("feature_disabled"), Some(V::Bool(true))),
    )?;
    c.f(what, &m.value, &floats(get(r, "value_vec4")))?;
    c.eq(
        what,
        m.has_value,
        matches!(get(r, "has_value"), Some(V::Bool(true))),
    )
}

fn matrix(c: &mut Check, what: &str, m: &Matrix, r: Option<&V>) -> Result<(), String> {
    c.f(what, &m.0, &floats(r))
}

/// Compare `scene` with `canon` (a full canonical projection of the same
/// input); returns the number of values compared.
pub fn check(canon: &CanonScene<'_>, scene: &Scene, limits: &DumpLimits) -> Result<u64, String> {
    // The public slices follow ufbx's list order, not the canonical element
    // order, so every pairing below walks the elements in that order.
    let order = canon
        .typed_order(limits)
        .map_err(|e| format!("order: {e:?}"))?;
    let flat: Vec<usize> = order
        .iter()
        .flat_map(|(_, list)| list.iter().copied())
        .collect();
    let mut c = Check { values: 0 };
    let mut k = [0usize; 6];
    let mut slots = vec![None; canon.elements.len()];
    for (_, list) in order.iter().filter(|(ty, _)| *ty == T::Texture) {
        for (count, &i) in list.iter().enumerate() {
            slots[i] = Some(count);
        }
    }
    for &element in &flat {
        let e = &canon.elements[element];
        let o = &e.obj;
        match e.ty {
            T::Node => {
                let n = scene.nodes().get(k[0]).ok_or("missing node")?;
                k[0] += 1;
                c.eq("node.source_id", n.source_id(), e.source_id)?;
                matrix(
                    &mut c,
                    "node_to_world",
                    n.node_to_world(),
                    get(o, "node_to_world"),
                )?;
                matrix(
                    &mut c,
                    "node_to_parent",
                    n.node_to_parent(),
                    get(o, "node_to_parent"),
                )?;
                matrix(
                    &mut c,
                    "geometry_to_world",
                    n.geometry_to_world(),
                    get(o, "geometry_to_world"),
                )?;
                if let Some(V::Obj(t)) = get(o, "local_transform") {
                    let l = n.local_transform();
                    c.f(
                        "local.translation",
                        &l.translation,
                        &floats(get(t, "translation")),
                    )?;
                    c.f("local.rotation", &l.rotation, &floats(get(t, "rotation")))?;
                    c.f("local.scale", &l.scale, &floats(get(t, "scale")))?;
                }
            }
            T::Mesh => {
                let m = scene.meshes().get(k[1]).ok_or("missing mesh")?;
                k[1] += 1;
                c.eq("mesh.source_id", m.source_id(), e.source_id)?;
                if get(o, "vertex_position").is_none() {
                    // Conversion failed in both paths (warning printed).
                    c.eq("failed mesh", m.faces().len(), 0)?;
                    continue;
                }
                attrib(
                    &mut c,
                    "position",
                    Some(m.positions()),
                    get(o, "vertex_position"),
                )?;
                attrib(&mut c, "normal", m.normals(), get(o, "vertex_normal"))?;
                let sets = match get(o, "uv_sets") {
                    Some(V::List(l)) => l.as_slice(),
                    _ => &[],
                };
                c.eq("uv_sets", m.uv_sets().len(), sets.len())?;
                for (u, r) in m.uv_sets().iter().zip(sets) {
                    let V::Obj(r) = r else { continue };
                    attrib(&mut c, "uv", Some(u.uv()), get(r, "vertex_uv"))?;
                    attrib(&mut c, "tangent", u.tangent(), get(r, "vertex_tangent"))?;
                    attrib(
                        &mut c,
                        "bitangent",
                        u.bitangent(),
                        get(r, "vertex_bitangent"),
                    )?;
                }
                let sets = match get(o, "color_sets") {
                    Some(V::List(l)) => l.as_slice(),
                    _ => &[],
                };
                c.eq("color_sets", m.color_sets().len(), sets.len())?;
                for (s, r) in m.color_sets().iter().zip(sets) {
                    let V::Obj(r) = r else { continue };
                    attrib(&mut c, "color", Some(s.color()), get(r, "vertex_color"))?;
                }
                c.u(
                    "face_material",
                    m.face_material(),
                    &u32s(get(o, "face_material")),
                )?;
                let parts = match get(o, "material_parts") {
                    Some(V::List(l)) => l.as_slice(),
                    _ => &[],
                };
                c.eq("parts", m.parts().len(), parts.len())?;
                for (p, r) in m.parts().iter().zip(parts) {
                    let V::Obj(r) = r else { continue };
                    c.u(
                        "part faces",
                        p.face_indices(),
                        &u32s(get(r, "face_indices")),
                    )?;
                }
                let tris: u64 = m.parts().iter().map(|p| p.num_triangles()).sum();
                c.eq("triangle count", m.triangles().len() as u64, tris * 3)?;
            }
            T::Material => {
                let m = scene.materials().get(k[2]).ok_or("missing material")?;
                k[2] += 1;
                for (key, maps) in [
                    ("fbx", m.fbx_maps().collect::<Vec<_>>()),
                    ("pbr", m.pbr_maps().collect()),
                ] {
                    let Some(V::Obj(r)) = get(o, key) else {
                        return Err(format!("material {key}: no record"));
                    };
                    c.eq("map count", maps.len(), r.0.len())?;
                    for (name, mm) in maps {
                        let Some(V::Obj(x)) = get(r, name) else {
                            return Err(format!("map {name}: no record"));
                        };
                        map(&mut c, name, mm, x, &slots)?;
                    }
                }
                // Features and attachments must not disappear during public projection.
                let Some(V::Obj(features)) = o.get("features") else {
                    return Err("missing material features".into());
                };
                c.eq("feature count", m.features().count(), features.0.len())?;
                for (name, feature) in m.features() {
                    let Some(V::Obj(f)) = features.get(name) else {
                        return Err("missing feature".into());
                    };
                    c.eq(
                        "feature enabled",
                        feature.enabled,
                        matches!(f.get("enabled"), Some(V::Bool(true))),
                    )?;
                    c.eq(
                        "feature explicit",
                        feature.is_explicit,
                        matches!(f.get("is_explicit"), Some(V::Bool(true))),
                    )?;
                }
                let attachments = list(o.get("textures"));
                c.eq("attachment count", m.textures().len(), attachments.len())?;
                for (a, r) in m.textures().iter().zip(attachments) {
                    let V::Obj(r) = r else {
                        return Err("attachment record".into());
                    };
                    c.eq(
                        "material property",
                        a.material_prop.as_slice(),
                        bytes(r.get("material_prop")),
                    )?;
                    c.eq(
                        "shader property",
                        a.shader_prop.as_slice(),
                        bytes(r.get("shader_prop")),
                    )?;
                    c.eq(
                        "attachment texture",
                        Some(a.texture),
                        texture_ref(r.get("texture"), &slots),
                    )?;
                }
                // Canonical headers retain authored and default property layers.
                let mut properties = std::collections::BTreeMap::new();
                let source_props = match o.get("element") {
                    Some(V::Obj(header)) => header.get("props"),
                    _ => None,
                };
                if let Some(V::Obj(layers)) = source_props {
                    if let Some(V::Obj(defaults)) = layers.get("defaults") {
                        for p in list(defaults.get("props")) {
                            if let V::Obj(p) = p {
                                properties.insert(bytes(p.get("name")), p);
                            }
                        }
                    }
                    for p in list(layers.get("props")) {
                        if let V::Obj(p) = p {
                            properties.insert(bytes(p.get("name")), p);
                        }
                    }
                }
                c.eq(
                    "source property count",
                    m.source_properties().len(),
                    properties.len(),
                )?;
                for p in m.source_properties() {
                    let r = properties
                        .get(p.name.as_slice())
                        .ok_or("source property missing")?;
                    c.eq(
                        "property type",
                        i64::from(p.property_type),
                        integer(r.get("type")),
                    )?;
                    c.eq(
                        "property flags",
                        i64::from(p.flags),
                        integer(r.get("flags")),
                    )?;
                    c.eq("property integer", p.value_int, integer(r.get("value_int")))?;
                    c.f(
                        "property real",
                        &p.value_real,
                        &floats(r.get("value_real_arr")),
                    )?;
                    c.eq(
                        "property string",
                        p.value_str.as_slice(),
                        bytes(r.get("value_str")),
                    )?;
                    c.eq(
                        "property blob",
                        p.value_blob.as_slice(),
                        bytes(r.get("value_blob")),
                    )?;
                }
            }
            T::Texture => {
                let t = scene.textures().get(k[3]).ok_or("missing texture")?;
                k[3] += 1;
                let content = match get(o, "content") {
                    Some(V::Bytes(b)) => b.as_slice(),
                    _ => &[],
                };
                c.eq("texture content", t.content(), content)?;
                c.eq(
                    "layered texture",
                    t.is_layered(),
                    integer(o.get("type")) == 1,
                )?;
                let layers = list(o.get("layers"));
                c.eq("texture layer count", t.layers().len(), layers.len())?;
                for (a, r) in t.layers().iter().zip(layers) {
                    let V::Obj(r) = r else {
                        return Err("texture layer record".into());
                    };
                    c.eq(
                        "layer texture",
                        Some(a.texture),
                        texture_ref(r.get("texture"), &slots),
                    )?;
                    c.eq("layer blend", a.blend_mode, integer(r.get("blend_mode")))?;
                    c.f("layer alpha", &[a.alpha], &floats(r.get("alpha")))?;
                }
                let files: Vec<_> = list(o.get("file_textures"))
                    .iter()
                    .map(|v| texture_ref(Some(v), &slots).unwrap())
                    .collect();
                c.eq("file textures", t.file_textures(), files.as_slice())?;
            }
            T::Camera => {
                let cam = scene.cameras().get(k[4]).ok_or("missing camera")?;
                k[4] += 1;
                c.f(
                    "fov",
                    &cam.field_of_view_deg(),
                    &floats(get(o, "field_of_view_deg")),
                )?;
            }
            T::Light => {
                let l = scene.lights().get(k[5]).ok_or("missing light")?;
                k[5] += 1;
                c.f(
                    "light dir",
                    &l.local_direction(),
                    &floats(get(o, "local_direction")),
                )?;
                c.f(
                    "light intensity",
                    &[l.intensity()],
                    &floats(get(o, "intensity")),
                )?;
            }
            _ => {}
        }
    }
    c.eq("node count", scene.nodes().len(), k[0])?;
    c.eq("mesh count", scene.meshes().len(), k[1])?;
    c.eq("material count", scene.materials().len(), k[2])?;
    c.eq("texture count", scene.textures().len(), k[3])?;
    c.eq("camera count", scene.cameras().len(), k[4])?;
    c.eq("light count", scene.lights().len(), k[5])?;
    c.f(
        "unit_meters",
        &[scene.settings().unit_meters()],
        &floats(canon.settings.get("unit_meters")),
    )?;
    c.eq(
        "unsupported",
        scene.unsupported().len(),
        canon.unsupported.len(),
    )?;
    let by_type: std::collections::BTreeMap<T, Vec<usize>> = order.iter().cloned().collect();
    let elems = by_type.get(&T::Bone).cloned().unwrap_or_default();
    c.eq("bones count", scene.bones().len(), elems.len())?;
    for (p, &i) in scene.bones().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::List(
                p.instances
                    .iter()
                    .map(|&k| V::Ref(Some(by_type[&T::Node][k])))
                    .collect(),
            ),
            match e.obj.get("element") {
                Some(V::Obj(o)) => o.get("instances"),
                _ => None,
            }
            .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::F(p.radius),
            e.obj.get("radius").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::F(p.relative_length),
            e.obj
                .get("relative_length")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Bool(p.is_root),
            e.obj.get("is_root").ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::SkinDeformer).cloned().unwrap_or_default();
    c.eq(
        "skin_deformers count",
        scene.skin_deformers().len(),
        elems.len(),
    )?;
    for (p, &i) in scene.skin_deformers().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::I(match p.skinning_method {
                fbx_rs::SkinningMethod::Linear => 0,
                fbx_rs::SkinningMethod::Rigid => 1,
                fbx_rs::SkinningMethod::DualQuaternion => 2,
                fbx_rs::SkinningMethod::BlendedDqLinear => 3,
                _ => unreachable!(),
            }),
            e.obj
                .get("skinning_method")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.clusters
                    .iter()
                    .map(|&k| V::Ref(Some(by_type[&T::SkinCluster][k])))
                    .collect(),
            ),
            e.obj.get("clusters").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.vertices
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
            e.obj.get("vertices").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.weights
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
            e.obj.get("weights").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::U(p.max_weights_per_vertex as u64),
            e.obj
                .get("max_weights_per_vertex")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(p.dq_vertices.iter().map(|&v| V::U(v.into())).collect()),
            e.obj.get("dq_vertices").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::F64s(p.dq_weights.clone()),
            e.obj.get("dq_weights").ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::SkinCluster).cloned().unwrap_or_default();
    c.eq(
        "skin_clusters count",
        scene.skin_clusters().len(),
        elems.len(),
    )?;
    for (p, &i) in scene.skin_clusters().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::Ref(p.bone_node.map(|k| by_type[&T::Node][k])),
            e.obj.get("bone_node").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Vec(p.geometry_to_bone.0.to_vec()),
            e.obj
                .get("geometry_to_bone")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Vec(p.mesh_node_to_bone.0.to_vec()),
            e.obj
                .get("mesh_node_to_bone")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Vec(p.bind_to_world.0.to_vec()),
            e.obj
                .get("bind_to_world")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Vec(p.geometry_to_world.0.to_vec()),
            e.obj
                .get("geometry_to_world")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Obj(
                Obj::new()
                    .with(
                        "translation",
                        V::Vec(p.geometry_to_world_transform.translation.to_vec()),
                    )
                    .with(
                        "rotation",
                        V::Vec(p.geometry_to_world_transform.rotation.to_vec()),
                    )
                    .with(
                        "scale",
                        V::Vec(p.geometry_to_world_transform.scale.to_vec()),
                    ),
            ),
            e.obj
                .get("geometry_to_world_transform")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(p.vertices.iter().map(|&v| V::U(v.into())).collect()),
            e.obj.get("vertices").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::F64s(p.weights.clone()),
            e.obj.get("weights").ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::BlendDeformer).cloned().unwrap_or_default();
    c.eq(
        "blend_deformers count",
        scene.blend_deformers().len(),
        elems.len(),
    )?;
    for (p, &i) in scene.blend_deformers().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::List(
                p.channels
                    .iter()
                    .map(|&k| V::Ref(Some(by_type[&T::BlendChannel][k])))
                    .collect(),
            ),
            e.obj.get("channels").ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::BlendChannel).cloned().unwrap_or_default();
    c.eq(
        "blend_channels count",
        scene.blend_channels().len(),
        elems.len(),
    )?;
    for (p, &i) in scene.blend_channels().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::F(p.weight),
            e.obj.get("weight").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.keyframes
                    .iter()
                    .map(|v| {
                        V::Obj(
                            Obj::new()
                                .with("shape", V::Ref(Some(by_type[&T::BlendShape][v.shape])))
                                .with("target_weight", V::F(v.target_weight))
                                .with("effective_weight", V::F(v.effective_weight)),
                        )
                    })
                    .collect(),
            ),
            e.obj.get("keyframes").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::Ref(p.target_shape.map(|k| by_type[&T::BlendShape][k])),
            e.obj.get("target_shape").ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::BlendShape).cloned().unwrap_or_default();
    c.eq(
        "blend_shapes count",
        scene.blend_shapes().len(),
        elems.len(),
    )?;
    for (p, &i) in scene.blend_shapes().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::List(p.offset_vertices.iter().map(|&v| V::U(v.into())).collect()),
            e.obj
                .get("offset_vertices")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.position_offsets
                    .iter()
                    .map(|v| V::Vec(v.to_vec()))
                    .collect(),
            ),
            e.obj
                .get("position_offsets")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.normal_offsets
                    .iter()
                    .map(|v| V::Vec(v.to_vec()))
                    .collect(),
            ),
            e.obj
                .get("normal_offsets")
                .ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::F64s(p.offset_weights.clone()),
            e.obj
                .get("offset_weights")
                .ok_or("missing deformer record")?,
        )?;
    }
    let elems = by_type.get(&T::Pose).cloned().unwrap_or_default();
    c.eq("poses count", scene.poses().len(), elems.len())?;
    for (p, &i) in scene.poses().iter().zip(&elems) {
        let e = &canon.elements[i];
        c.eq(
            "deformer name",
            p.name.as_str(),
            String::from_utf8_lossy(&e.name).as_ref(),
        )?;
        let source_id = if e.node.is_some_and(|n| canon.doc.nodes[n].name == b"Shape") {
            None
        } else {
            e.source_id
        };
        c.eq("deformer source ID", p.source_id, source_id)?;
        deformer_value(
            &mut c,
            &V::Bool(p.is_bind_pose),
            e.obj.get("is_bind_pose").ok_or("missing deformer record")?,
        )?;
        deformer_value(
            &mut c,
            &V::List(
                p.bone_poses
                    .iter()
                    .map(|v| {
                        V::Obj(
                            Obj::new()
                                .with("bone_node", V::Ref(Some(by_type[&T::Node][v.bone_node])))
                                .with("bone_to_world", V::Vec(v.bone_to_world.0.to_vec()))
                                .with("bone_to_parent", V::Vec(v.bone_to_parent.0.to_vec())),
                        )
                    })
                    .collect(),
            ),
            e.obj.get("bone_poses").ok_or("missing deformer record")?,
        )?;
    }
    for (mesh, &i) in scene
        .meshes()
        .iter()
        .zip(by_type.get(&T::Mesh).into_iter().flatten())
    {
        for (indices, ty, name) in [
            (mesh.skin_deformers(), T::SkinDeformer, "skin_deformers"),
            (mesh.blend_deformers(), T::BlendDeformer, "blend_deformers"),
        ] {
            let v = V::List(
                indices
                    .iter()
                    .map(|&k| V::Ref(Some(by_type[&ty][k])))
                    .collect(),
            );
            deformer_value(
                &mut c,
                &v,
                canon.elements[i]
                    .obj
                    .get(name)
                    .ok_or("missing mesh deformers")?,
            )?;
        }
    }
    for (node, &i) in scene.nodes().iter().zip(&by_type[&T::Node]) {
        for (index, ty, name) in [
            (node.bone(), T::Bone, "bone"),
            (node.bind_pose(), T::Pose, "bind_pose"),
        ] {
            let v = V::Ref(index.map(|k| by_type[&ty][k]));
            deformer_value(
                &mut c,
                &v,
                canon.elements[i].obj.get(name).unwrap_or(&V::Ref(None)),
            )?;
        }
    }
    check_animation(&mut c, canon, scene)?;
    Ok(c.values)
}

fn deformer_value(c: &mut Check, a: &V, b: &V) -> Result<(), String> {
    match (a, b) {
        (V::F(a), V::F(b)) => c.f("deformer float", &[*a], &[*b]),
        (V::Vec(a), V::Vec(b)) | (V::F64s(a), V::F64s(b)) => c.f("deformer array", a, b),
        (V::List(a), V::List(b)) => {
            c.eq("deformer list length", a.len(), b.len())?;
            for (a, b) in a.iter().zip(b) {
                deformer_value(c, a, b)?;
            }
            Ok(())
        }
        (V::Obj(a), V::Obj(b)) => {
            for (k, v) in &a.0 {
                deformer_value(
                    c,
                    v,
                    b.get(k)
                        .ok_or_else(|| format!("missing deformer field {k}"))?,
                )?;
            }
            Ok(())
        }
        _ => c.eq("deformer value", a, b),
    }
}

fn check_animation(c: &mut Check, canon: &CanonScene<'_>, scene: &Scene) -> Result<(), String> {
    use fbx_rs::canon_scene as cs;
    c.eq(
        "animation stacks",
        scene.animation_stacks(),
        cs::animation_stacks(canon),
    )?;
    c.eq(
        "animation layers",
        scene.animation_layers(),
        cs::animation_layers(canon),
    )?;
    c.eq(
        "animation values",
        scene.animation_values(),
        cs::animation_values(canon),
    )?;
    c.eq(
        "animation curve count",
        scene.animation_curves().len(),
        cs::animation_curves(canon).len(),
    )?;
    for (a, b) in scene
        .animation_stacks()
        .iter()
        .zip(cs::animation_stacks(canon))
    {
        c.f(
            "take bounds",
            &[a.time_begin, a.time_end],
            &[b.time_begin, b.time_end],
        )?;
    }
    for (a, b) in scene
        .animation_layers()
        .iter()
        .zip(cs::animation_layers(canon))
    {
        c.f("layer weight", &[a.weight], &[b.weight])?;
    }
    for (a, b) in scene
        .animation_values()
        .iter()
        .zip(cs::animation_values(canon))
    {
        c.f("animation defaults", &a.default_value, &b.default_value)?;
    }
    for (a, b) in scene
        .animation_curves()
        .iter()
        .zip(cs::animation_curves(canon))
    {
        c.eq("animation curve", a, b)?;
        for (a, b) in a.keys.iter().zip(&b.keys) {
            c.f(
                "curve key",
                &[
                    a.time,
                    a.value,
                    f64::from(a.left.dx),
                    f64::from(a.left.dy),
                    f64::from(a.right.dx),
                    f64::from(a.right.dy),
                ],
                &[
                    b.time,
                    b.value,
                    f64::from(b.left.dx),
                    f64::from(b.left.dy),
                    f64::from(b.right.dx),
                    f64::from(b.right.dy),
                ],
            )?;
        }
    }
    for stack in 0..scene.animation_stacks().len().max(1) {
        let stack = (!scene.animation_stacks().is_empty()).then_some(stack);
        for time in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0] {
            let a = scene.evaluate(stack, time).map_err(|e| e.to_string())?;
            let b = cs::evaluate_pose(canon, stack, time).map_err(|e| e.to_string())?;
            c.eq("evaluated node count", a.nodes.len(), b.nodes.len())?;
            for (a, b) in a.nodes.iter().zip(&b.nodes) {
                c.f(
                    "evaluated translation",
                    &a.local_transform.translation,
                    &b.local_transform.translation,
                )?;
                c.f(
                    "evaluated rotation",
                    &a.local_transform.rotation,
                    &b.local_transform.rotation,
                )?;
                c.f(
                    "evaluated scale",
                    &a.local_transform.scale,
                    &b.local_transform.scale,
                )?;
                for (a, b) in [
                    (&a.node_to_parent, &b.node_to_parent),
                    (&a.node_to_world, &b.node_to_world),
                    (&a.geometry_to_node, &b.geometry_to_node),
                    (&a.geometry_to_world, &b.geometry_to_world),
                ] {
                    c.f("evaluated matrix", &a.0, &b.0)?;
                }
            }
            c.f(
                "evaluated blend weights",
                &a.blend_weights,
                &b.blend_weights,
            )?;
            c.eq(
                "evaluated cluster count",
                a.skin_matrices.len(),
                b.skin_matrices.len(),
            )?;
            for (a, b) in a.skin_matrices.iter().zip(&b.skin_matrices) {
                c.f("evaluated cluster", &a.0, &b.0)?;
            }
            for layer in scene.animation_layers() {
                for binding in &layer.bindings {
                    let a = scene
                        .evaluate_binding(stack, binding, time)
                        .map_err(|e| e.to_string())?;
                    let b = cs::evaluate_binding(canon, stack, binding, time)
                        .map_err(|e| e.to_string())?;
                    c.f("evaluated property", &a.components, &b.components)?;
                    c.eq("evaluated property metadata", a, b)?;
                }
            }
        }
    }
    Ok(())
}
