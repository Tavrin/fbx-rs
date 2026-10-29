//! Cross-check of the public [`fbx_rs::Scene`] against the canonical
//! projection that the corpus comparison proves equal to ufbx: every exposed
//! value must match its canonical record bit for bit.

use fbx_rs::canon::{Obj, V};
use fbx_rs::canon_scene::{CanonScene, ElementType as T};
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

fn map(c: &mut Check, what: &str, m: &MaterialMap, r: &Obj) -> Result<(), String> {
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
pub fn check(canon: &CanonScene<'_>, scene: &Scene) -> Result<u64, String> {
    let mut c = Check { values: 0 };
    let mut k = [0usize; 6];
    for e in &canon.elements {
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
                        map(&mut c, name, mm, x)?;
                    }
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
    Ok(c.values)
}
