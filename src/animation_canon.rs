//! Canonical animation samples, matching reference/canon.c.
use crate::animation::*;
use crate::canon::{Emitter, Obj, K, V};
use crate::canon_scene::CanonScene;
use std::io::{self, Write};
#[derive(Clone)]
struct Sample {
    time: f64,
    kind: &'static str,
    curve: Option<usize>,
    index: usize,
    num: i64,
    den: i64,
}
fn next_after(x: f64, up: bool) -> f64 {
    if x == 0.0 {
        return if up {
            f64::from_bits(1)
        } else {
            -f64::from_bits(1)
        };
    }
    f64::from_bits(if (x > 0.0) == up {
        x.to_bits() + 1
    } else {
        x.to_bits() - 1
    })
}
fn add(samples: &mut Vec<Sample>, s: Sample) -> io::Result<()> {
    if samples.len() >= 200_000 || !s.time.is_finite() {
        return Err(io::Error::other("animation sample limit or nonfinite time"));
    }
    samples.push(s);
    Ok(())
}
fn curve_samples(samples: &mut Vec<Sample>, c: &AnimationCurve) -> io::Result<()> {
    for (i, k) in c.keys.iter().enumerate() {
        let mut put = |time, kind, num, den| {
            add(
                samples,
                Sample {
                    time,
                    kind,
                    curve: Some(c.element),
                    index: i,
                    num,
                    den,
                },
            )
        };
        put(k.time, "key", 0, 1)?;
        put(next_after(k.time, false), "seconds-before-key", -1, 0)?;
        put(next_after(k.time, true), "seconds-after-key", 1, 0)?;
        if k.ticks > i64::MIN {
            put(
                (k.ticks - 1) as f64 / KTIME,
                "tick-before-key",
                k.ticks - 1,
                KTIME as i64,
            )?;
        }
        if k.ticks < i64::MAX {
            put(
                (k.ticks + 1) as f64 / KTIME,
                "tick-after-key",
                k.ticks + 1,
                KTIME as i64,
            )?;
        }
        if let Some(next) = c.keys.get(i + 1).filter(|n| n.time > k.time) {
            for j in 1..=3 {
                put(
                    k.time + (next.time - k.time) * (j as f64 / 4.0),
                    "segment-fraction",
                    j,
                    4,
                )?;
            }
        }
    }
    Ok(())
}
fn take_samples(samples: &mut Vec<Sample>, begin: f64, end: f64) -> io::Result<()> {
    if !begin.is_finite() || !end.is_finite() || end < begin {
        return Err(io::Error::other("invalid take bounds"));
    }
    let mut put = |time, kind, num, den| {
        add(
            samples,
            Sample {
                time,
                kind,
                curve: None,
                index: 0,
                num,
                den,
            },
        )
    };
    put(next_after(begin, false), "seconds-before-take", -1, 0)?;
    put(next_after(end, true), "seconds-after-take", 1, 0)?;
    put(begin - 1.0, "before-take", -1, 1)?;
    put(end + 1.0, "after-take", 1, 1)?;
    let frames = ((end - begin) * 30.0).ceil();
    if !frames.is_finite() || frames > 200_000.0 {
        return Err(io::Error::other("30 Hz grid work limit"));
    }
    for i in 0..frames as i64 {
        put(begin + i as f64 / 30.0, "grid-from-take-start", i, 30)?;
    }
    put(end, "take-end", 0, 1)
}
fn dump_sample<W: Write>(em: &mut Emitter<'_, W>, s: &Sample) -> io::Result<()> {
    let curve_key = s.curve.map(|i| em.keys[i].clone());
    em.leaf_raw("time", |b| crate::canon::f64_into(b, s.time))?;
    em.leaf_raw("provenance", |b| {
        b.extend_from_slice(format!("{{\"kind\":\"{}\",\"curve\":", s.kind).as_bytes());
        if let Some(key) = &curve_key {
            b.extend_from_slice(format!("{{\"ref\":\"{key}\"}}").as_bytes());
        } else {
            b.extend_from_slice(b"null");
        }
        b.extend_from_slice(
            format!(
                ",\"key_index\":\"{}\",\"numerator\":\"{}\",\"denominator\":\"{}\"}}",
                s.index, s.num, s.den
            )
            .as_bytes(),
        );
    })
}
fn eval<W: Write>(
    em: &mut Emitter<'_, W>,
    name: &str,
    tag: &str,
    values: &[f64],
) -> io::Result<()> {
    if values.iter().any(|v| !v.is_finite()) {
        return Err(io::Error::other("nonfinite evaluated value"));
    }
    em.leaf_raw(name, |b| {
        b.extend_from_slice(format!("{{\"{tag}\":[").as_bytes());
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                b.push(b',');
            }
            b.extend_from_slice(format!("\"{:016x}\"", v.to_bits()).as_bytes());
        }
        b.extend_from_slice(b"]}");
    })
}
fn dump_prop<W: Write>(
    em: &mut Emitter<'_, W>,
    a: &Animation,
    layers: &[usize],
    b: &AnimationBinding,
    time: f64,
) -> io::Result<()> {
    let p = a.property(layers, b.element, &b.property, time);
    let m = em.push("binding");
    em.emit_struct(
        "ufbx_anim_prop",
        Some(
            &Obj::new()
                .with("element", V::Ref(Some(b.element)))
                .with("prop_name", crate::canon::s(&b.property))
                .with("anim_value", V::Ref(Some(a.values[b.value].element))),
        ),
    )?;
    em.restore(m);
    for (name, ty, v) in [
        ("type", "ufbx_prop_type", u64::from(p.property_type)),
        ("flags", "ufbx_prop_flags", u64::from(p.flags)),
    ] {
        em.leaf_raw(name, |b| {
            b.extend_from_slice(format!("{{\"enum\":\"{ty}\",\"i64\":\"{v}\"}}").as_bytes())
        })?;
    }
    em.leaf_raw("value_int", |b| {
        b.extend_from_slice(format!("{{\"i64\":\"{}\"}}", p.value_int).as_bytes())
    })?;
    // Use the generated property walker to encode strings/blobs identically.
    // Its leaf encodings are also exposed by the small value structs.
    let m = em.push("value_str");
    em.leaf_here(K::Str, Some(&crate::canon::s(&p.value_str)))?;
    em.restore(m);
    let m = em.push("value_blob");
    em.leaf_here(K::Blob, Some(&crate::canon::s(&p.value_blob)))?;
    em.restore(m);
    eval(em, "components", "eval_f64", &p.components)
}
pub(crate) fn write<W: Write>(em: &mut Emitter<'_, W>, scene: &CanonScene<'_>) -> io::Result<()> {
    let a = &scene.animation;
    let default = a.default_stack.map_or((0.0, 0.0), |i| {
        (a.stacks[i].time_begin, a.stacks[i].time_end)
    });
    em.begin("curve_samples")?;
    em.count("length", a.curves.len())?;
    for c in &a.curves {
        let base = em.push(&scene.elements[c.element].key);
        let mut samples = Vec::new();
        curve_samples(&mut samples, c)?;
        take_samples(&mut samples, default.0, default.1)?;
        em.count("length", samples.len())?;
        for (i, s) in samples.iter().enumerate() {
            let m = em.push_index(i);
            dump_sample(em, s)?;
            eval(em, "value", "eval_f64", &[c.eval(s.time, 0.0, true)])?;
            em.restore(m);
        }
        em.restore(base);
    }
    em.end()?;
    em.begin("evaluated")?;
    em.count("length", a.stacks.len().max(1))?;
    for (key, begin, end) in &scene.takes {
        let stack = a
            .stacks
            .iter()
            .find(|s| scene.elements[s.element].key == *key);
        let layers = stack.map_or(&[][..], |s| s.layers.as_slice());
        let mut samples = Vec::new();
        for &ix in layers {
            for &v in &a.layers[ix].values {
                for c in a.values[v].curves.iter().flatten() {
                    curve_samples(&mut samples, &a.curves[*c])?;
                }
            }
        }
        take_samples(&mut samples, *begin, *end)?;
        samples.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap()
                .then(a.time.to_bits().cmp(&b.time.to_bits()))
                .then_with(|| {
                    a.curve
                        .map_or("", |i| scene.elements[i].key.as_str())
                        .cmp(b.curve.map_or("", |i| scene.elements[i].key.as_str()))
                })
                .then(a.kind.cmp(b.kind))
                .then(a.index.cmp(&b.index))
                .then(a.num.cmp(&b.num))
                .then(a.den.cmp(&b.den))
        });
        samples.dedup_by(|b, a| a.time.to_bits() == b.time.to_bits());
        if !scene.elements.is_empty() && samples.len() > 10_000_000 / scene.elements.len() {
            return Err(io::Error::other("scene evaluation work limit"));
        }
        let base = em.push(key);
        em.count("length", samples.len())?;
        for (s_ix, s) in samples.iter().enumerate() {
            let m = em.push_index(s_ix);
            dump_sample(em, s)?;
            for (l, &ix) in layers.iter().enumerate() {
                let lm = em.push("layers");
                let li = em.push_index(l);
                let mut bindings: Vec<_> = a.layers[ix].bindings.iter().collect();
                bindings.sort_by(|a, b| {
                    scene.elements[a.element]
                        .key
                        .cmp(&scene.elements[b.element].key)
                        .then(a.property.cmp(&b.property))
                });
                for (p, b) in bindings.iter().enumerate() {
                    let pm = em.push_index(p);
                    let cm = em.push("layer");
                    dump_prop(em, a, &[ix], b, s.time)?;
                    em.restore(cm);
                    let cm = em.push("composed");
                    dump_prop(em, a, layers, b, s.time)?;
                    em.restore(cm);
                    em.restore(pm);
                }
                em.restore(li);
                em.restore(lm);
            }
            let states = a.states(layers, s.time);
            let nm = em.push("nodes");
            em.count("length", states.len())?;
            for (i, st) in states.iter().enumerate() {
                let k = em.push(&scene.elements[a.nodes[i].element].key);
                for (part, t) in [("local", &st.local), ("geometry", &st.geometry)] {
                    let p = em.push(part);
                    eval(em, "translation", "eval_f64", &t.translation)?;
                    eval(em, "rotation", "eval_quat", &t.rotation)?;
                    eval(em, "scale", "eval_f64", &t.scale)?;
                    em.restore(p);
                }
                for (name, mat) in [
                    ("node_to_parent", &st.node_to_parent),
                    ("node_to_world", &st.node_to_world),
                    ("geometry_to_node", &st.geometry_to_node),
                    ("geometry_to_world", &st.geometry_to_world),
                ] {
                    eval(em, name, "eval_f64", mat)?;
                }
                em.restore(k);
            }
            em.restore(nm);
            let sk = em.push("skins");
            em.count("length", a.clusters.len())?;
            for ((element, c), (bone, bind)) in scene.deformers.clusters.iter().zip(&a.clusters) {
                let k = em.push(&scene.elements[*element].key);
                let mat = bone.map_or(c.geometry_to_world.0, |i| {
                    crate::scene_node::matrix_mul(&states[i].node_to_world, bind)
                });
                eval(em, "geometry_to_world", "eval_f64", &mat)?;
                em.restore(k);
            }
            em.restore(sk);
            let mo = em.push("morphs");
            em.count("length", a.channels.len())?;
            for &element in &a.channels {
                let k = em.push(&scene.elements[element].key);
                eval(
                    em,
                    "weight",
                    "eval_f64",
                    &[a.property(layers, element, b"DeformPercent", s.time)
                        .components[0]
                        * 0.01],
                )?;
                em.restore(k);
            }
            em.restore(mo);
            em.restore(m);
        }
        em.restore(base);
    }
    em.end()
}
