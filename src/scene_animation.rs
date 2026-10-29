//! Curve reconstruction and property bindings from pinned ufbx 0.23.1 (MIT).
use crate::animation::*;
use crate::canon::{s, Obj, V};
use crate::canon_scene::{CanonScene, DumpLimits, ElementType as T};
use crate::conversion_budget::{Budget, Path};
use crate::scene_property;
use std::collections::{BTreeMap, BTreeSet};

fn auto_tangent(
    times: [f64; 3],
    values: [f64; 3],
    weights: [f32; 2],
    bias: f32,
    flags: u32,
) -> f32 {
    let [prev, time, next] = times;
    let [pv, v, nv] = values;
    if flags & 0x1000 != 0 && (pv - v).abs().min((nv - v).abs()) <= 0.0 {
        return 0.0;
    }
    let mut slope = (nv - pv) / (next - prev);
    if flags & 0x2000 == 0 {
        let left = (v - pv) / (time - prev);
        let right = (nv - v) / (next - time);
        let delta = (time - prev) / (next - prev);
        slope = slope * 0.5 + (left * (1.0 - delta) + right * delta) * 0.5;
        let bw = f64::from(bias).abs() / 100.0;
        if bw > 0.0001 {
            let target = if bias > 0.0 { right } else { left };
            let bd = target - slope;
            slope = slope * (1.0 - bw) + target * bw;
            let abw = bw - 5.0;
            if abw > 0.0 {
                let sign = if bd.abs() > 0.00001 {
                    bd
                } else {
                    f64::from(bias)
                };
                let sign = if sign > 0.0 { 1.0 } else { -1.0 };
                slope += abw * abw * sign * 40.0;
            }
        }
    }
    if flags & 0x4000 != 0 {
        let sign = if slope >= 0.0 { 1.0 } else { -1.0 };
        let mut abs = sign * slope;
        let rl = f64::from(weights[0]) * (time - prev);
        let rr = f64::from(weights[1]) * (next - time);
        let ml = if rl > 0.0 { sign * (v - pv) / rl } else { 0.0 };
        let mr = if rr > 0.0 { sign * (nv - v) / rr } else { 0.0 };
        let ml = if ml > 0.0 { ml } else { 0.0 };
        let mr = if mr > 0.0 { mr } else { 0.0 };
        if abs > ml {
            abs = ml;
        }
        if abs > mr {
            abs = mr;
        }
        slope = sign * abs;
    }
    crate::numeric::round_f32(slope)
}
fn auto_edge(t0: f64, t1: f64, v0: f64, v1: f64, bias: f32, flags: u32) -> f32 {
    if flags & 0x4000 != 0 || (flags & 0x1000 != 0 && (v1 - v0).abs() <= 0.0) {
        return 0.0;
    }
    let mut slope = (v1 - v0) / (t1 - t0);
    if flags & 0x2000 == 0 {
        let b = f64::from(bias).abs() / 100.0 - 5.0;
        if b > 0.0 {
            slope += b * b * if bias > 0.0 { 1.0 } else { -1.0 } * 40.0;
        }
    }
    crate::numeric::round_f32(slope)
}
struct Reader<'s, 'd> {
    s: &'s CanonScene<'d>,
    children: Vec<Vec<usize>>,
    budget: Budget,
}
impl Reader<'_, '_> {
    fn child(&self, n: usize, name: &[u8]) -> Option<usize> {
        self.children[n]
            .iter()
            .copied()
            .find(|&c| self.s.doc.nodes[c].name == name)
    }
    fn array(&self, n: usize, name: &[u8], width: usize, limit: usize) -> Result<Vec<u8>, String> {
        let c = self
            .child(n, name)
            .ok_or("animation: missing curve array")?;
        self.budget
            .work(crate::canon_dom::data_len(&self.s.doc, c)?)?;
        for p in &self.s.doc.properties[self.s.doc.nodes[c].properties.clone()] {
            let count = match p {
                crate::syntax::Property::Array { count, .. } => *count,
                crate::syntax::Property::AsciiArray(v) => v.len(),
                _ => 0,
            };
            if count > limit / width {
                return Err("animation: array limit".into());
            }
        }
        if name == b"KeyTime" {
            use crate::syntax::{AsciiValue, Property};
            for p in &self.s.doc.properties[self.s.doc.nodes[c].properties.clone()] {
                match p {
                    Property::Array { kind, .. } if !matches!(kind, b'l' | b'i') => {
                        return Err("animation: key times require integer storage".into())
                    }
                    Property::F32(b) if !f32::from_bits(*b).is_finite() => {
                        return Err("animation: nonfinite key time".into())
                    }
                    Property::F64(b) if !f64::from_bits(*b).is_finite() => {
                        return Err("animation: nonfinite key time".into())
                    }
                    Property::AsciiArray(v)
                        if v.iter().any(|v| {
                            matches!(v,AsciiValue::F64(b) if !f64::from_bits(*b).is_finite())
                                || matches!(v, AsciiValue::Bare(_))
                        }) =>
                    {
                        return Err("animation: invalid key time".into())
                    }
                    _ => {}
                }
            }
        }

        let (_, bytes) = crate::canon_dom::animation_array(&self.s.doc, c, self.s.ascii)
            .ok_or("animation: invalid curve array")?;
        if bytes.len() > limit || bytes.len() % width != 0 {
            return Err("animation: array limit or type".into());
        }
        Ok(bytes)
    }
    fn ext(&self, n: usize, name: &[u8]) -> Result<Extrapolation, String> {
        use crate::canon_dom::{node_data, NodeData, Scalar};
        let mut out = Extrapolation {
            mode: ExtrapolationMode::Constant,
            repeat_count: -1,
        };
        let Some(n) = self.child(n, name) else {
            return Ok(out);
        };
        let number = |name| {
            self.child(n, name)
                .and_then(|c| match node_data(&self.s.doc, c, self.s.ascii) {
                    NodeData::Values(v) => v.first().and_then(|x| match x {
                        Scalar::Num(i, _) => Some(*i),
                        Scalar::Str(_) => None,
                    }),
                    _ => None,
                })
        };
        if let Some(c) = number(b"Type") {
            let c = i32::try_from(c).map_err(|_| "animation: extrapolation type out of range")?;
            out.mode = match c {
                65 => ExtrapolationMode::RepeatRelative,
                75 => ExtrapolationMode::Slope,
                77 => ExtrapolationMode::Mirror,
                82 => ExtrapolationMode::Repeat,
                _ => ExtrapolationMode::Constant,
            };
            out.repeat_count = i32::try_from(number(b"Repetition").unwrap_or(-1))
                .map_err(|_| "animation: extrapolation repetition out of range")?
                .max(-1);
        }
        Ok(out)
    }
    fn curve(
        &self,
        i: usize,
        n: usize,
        limit: usize,
        max_keys: usize,
    ) -> Result<AnimationCurve, String> {
        let ticks = self.array(n, b"KeyTime", 8, limit.min(max_keys.saturating_mul(8)))?;
        let values = self.array(n, b"KeyValueFloat", 8, limit)?;
        let flags = self.array(n, b"KeyAttrFlags", 4, limit)?;
        let attrs = self.array(n, b"KeyAttrDataFloat", 4, limit)?;
        let refs = self.array(n, b"KeyAttrRefCount", 4, limit)?;
        let count = ticks.len() / 8;
        if count > max_keys
            || count
                .checked_mul(std::mem::size_of::<Keyframe>())
                .is_none_or(|b| b > limit)
        {
            return Err("animation: key limit".into());
        }
        if values.len() != ticks.len()
            || flags.len() != refs.len()
            || attrs.len()
                != refs
                    .len()
                    .checked_mul(4)
                    .ok_or("animation: count overflow")?
        {
            return Err("animation: mismatched curve arrays".into());
        }
        let times: Vec<i64> = ticks
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| i64::from_le_bytes(*c))
            .collect();
        if times.windows(2).any(|p| p[0] > p[1]) {
            return Err("animation: unsorted key times".into());
        }
        let values: Vec<f64> = values
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err("animation: nonfinite key value".into());
        }
        let flags: Vec<u32> = flags
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        let attrs: Vec<f32> = attrs
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        let refs: Vec<i32> = refs
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes(*c))
            .collect();
        let mut total = 0usize;
        for &r in &refs {
            if r <= 0 {
                return Err("animation: invalid attribute repetition".into());
            }
            total = total
                .checked_add(
                    usize::try_from(r)
                        .map_err(|_| "animation: attribute repetition out of range")?,
                )
                .ok_or("animation: count overflow")?;
        }
        if total != count {
            return Err("animation: attribute count mismatch".into());
        }
        let mut keys = Vec::new();
        keys.try_reserve_exact(count)
            .map_err(|_| "animation: allocation")?;
        let (mut sl, mut wl) = (0.0f32, 0.333333f32);
        let mut prev = 0.0;
        let mut run = 0;
        let mut left = refs.first().copied().unwrap_or(0);
        for k in 0..count {
            let time = times[k] as f64 / KTIME;
            let next = times.get(k + 1).map_or(time, |&t| t as f64 / KTIME);
            let value = values[k];
            let f = flags[run];
            let a = &attrs[run * 4..run * 4 + 4];
            let (mut sr, mut wr, mut nsl, mut nwl) = (a[0], 0.333333f32, a[1], 0.333333f32);
            if f & 0x1000000 != 0 {
                wr = (a[2].to_bits() & 0xffff) as f32 * 0.0001f32;
            }
            if f & 0x2000000 != 0 {
                nwl = (a[2].to_bits() >> 16) as f32 * 0.0001f32;
            }
            let interpolation;
            if f & 2 != 0 {
                interpolation = if f & 0x100 != 0 {
                    Interpolation::ConstantNext
                } else {
                    Interpolation::ConstantPrevious
                };
                wr = 0.333333;
                nwl = 0.333333;
                sr = 0.0;
                nsl = 0.0;
            } else if f & 8 != 0 {
                interpolation = Interpolation::Cubic;
                if f & 0x200 != 0 {
                    let ls = if k > 0 && time > prev {
                        (value - values[k - 1]) / (time - prev)
                    } else {
                        0.0
                    };
                    let rs = if k + 1 < count && next > time {
                        (values[k + 1] - value) / (next - time)
                    } else {
                        0.0
                    };
                    let factor = if k == 0 || k + 1 == count || time <= prev || next <= time {
                        1.0
                    } else {
                        0.5
                    };
                    let (t, c, b) = (f64::from(a[0]), f64::from(a[1]), f64::from(a[2]));
                    let d00 = factor * (1.0 - t) * (1.0 + b) * (1.0 - c);
                    let d01 = factor * (1.0 - t) * (1.0 - b) * (1.0 + c);
                    let d10 = factor * (1.0 - t) * (1.0 + b) * (1.0 + c);
                    let d11 = factor * (1.0 - t) * (1.0 - b) * (1.0 - c);
                    sl = crate::numeric::round_f32(d00 * ls + d01 * rs);
                    sr = crate::numeric::round_f32(d10 * ls + d11 * rs);
                    nsl = 0.0;
                    nwl = 0.333333;
                } else if f & 0x400 == 0 {
                    if k > 0 && k + 1 < count && time > prev && next > time {
                        let ts = [prev, time, next];
                        let vs = [values[k - 1], value, values[k + 1]];
                        let ws = [wl, wr];
                        if (sl + sr).abs() <= 0.0001f32 {
                            sr = auto_tangent(ts, vs, ws, sr, f);
                            sl = sr;
                        } else {
                            sl = auto_tangent(ts, vs, ws, -sl, f);
                            sr = auto_tangent(ts, vs, ws, sr, f);
                        }
                    } else if k > 0 && time > prev {
                        sr = auto_edge(prev, time, values[k - 1], value, -sl, f);
                        sl = sr;
                    } else if k + 1 < count && next > time {
                        sr = auto_edge(time, next, value, values[k + 1], sr, f);
                        sl = sr;
                    } else {
                        sr = 0.0;
                        sl = 0.0;
                    }
                }
            } else {
                interpolation = Interpolation::Linear;
                wr = 0.333333;
                nwl = 0.333333;
                sr = if next > time {
                    crate::numeric::round_f32((values[k + 1] - value) / (next - time))
                } else {
                    0.0
                };
                nsl = sr;
            }
            let ld = if time > prev {
                crate::numeric::round_f32(f64::from(wl) * (time - prev))
            } else {
                0.0
            };
            let rd = if next > time {
                crate::numeric::round_f32(f64::from(wr) * (next - time))
            } else {
                0.0
            };
            let key = Keyframe {
                ticks: times[k],
                time,
                value,
                interpolation,
                left: Tangent {
                    dx: ld,
                    dy: if time > prev { ld * sl } else { 0.0 },
                },
                right: Tangent {
                    dx: rd,
                    dy: if next > time { rd * sr } else { 0.0 },
                },
            };
            if ![key.left.dx, key.left.dy, key.right.dx, key.right.dy]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err("animation: nonfinite tangent".into());
            }
            keys.push(key);
            sl = nsl;
            wl = nwl;
            prev = time;
            left -= 1;
            if left == 0 {
                run += 1;
                left = refs.get(run).copied().unwrap_or(0);
            }
        }
        Ok(AnimationCurve {
            element: i,
            source_id: self.s.elements[i].source_id.unwrap_or(0),
            keys,
            pre_extrapolation: self.ext(n, b"Pre-Extrapolation")?,
            post_extrapolation: self.ext(n, b"Post-Extrapolation")?,
        })
    }
}
fn property(p: &scene_property::AuthoredProperty<'_>) -> EvaluatedProperty {
    EvaluatedProperty {
        property_type: p.kind as u8,
        flags: p.flags,
        value_int: p.value_int,
        components: p.value_real,
        value_str: p.value_str.to_vec(),
        value_blob: p.value_blob.to_vec(),
    }
}
fn prop_obj(name: &[u8], p: &EvaluatedProperty) -> V {
    V::Obj(
        Obj::new()
            .with("name", s(name))
            .with("type", V::I(i64::from(p.property_type)))
            .with("flags", V::I(i64::from(p.flags)))
            .with("value_int", V::I(p.value_int))
            .with("value_real_arr", V::Vec(p.components.to_vec()))
            .with("value_str", s(&p.value_str))
            .with("value_blob", s(&p.value_blob)),
    )
}
fn binding_obj(b: &AnimationBinding, a: &Animation) -> Obj {
    Obj::new()
        .with("element", V::Ref(Some(b.element)))
        .with("prop_name", s(&b.property))
        .with("anim_value", V::Ref(Some(a.values[b.value].element)))
}
fn child_index(doc: &crate::syntax::Document<'_>) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); doc.nodes.len()];
    for (i, n) in doc.nodes.iter().enumerate() {
        if let Some(p) = n.parent {
            children[p].push(i);
        }
    }
    children
}
// Validate and charge every source array before either helper detection or key
// reconstruction can decode it. Repeated connections never repeat decoding.
fn preflight_curves(
    doc: &crate::syntax::Document<'_>,
    elements: &[crate::canon_scene::Element],
    children: &[Vec<usize>],
    limits: &DumpLimits,
    budget: &Budget,
) -> Result<(), String> {
    for e in elements {
        if e.ty != T::AnimCurve {
            continue;
        }
        let n = e.node.ok_or("animation: curve source")?;
        budget.work(children[n].len())?;
        let mut arrays = [None; 5];
        for &c in &children[n] {
            let slot = match doc.nodes[c].name {
                b"KeyTime" => 0,
                b"KeyValueFloat" => 1,
                b"KeyAttrFlags" => 2,
                b"KeyAttrDataFloat" => 3,
                b"KeyAttrRefCount" => 4,
                _ => continue,
            };
            if arrays[slot].is_none() {
                arrays[slot] = Some(c);
            }
        }
        let mut counts = [0; 5];
        for (i, node) in arrays.into_iter().enumerate() {
            let node = node.ok_or("animation: missing curve array")?;
            counts[i] = crate::canon_dom::data_len(doc, node)?;
        }
        if counts[0] > limits.max_keys {
            return Err("animation: key limit".into());
        }
        if counts[0] != counts[1]
            || counts[2] != counts[4]
            || counts[4].checked_mul(4) != Some(counts[3])
        {
            return Err("animation: mismatched curve arrays".into());
        }
        budget.charge(counts[0], std::mem::size_of::<Keyframe>() * 2)?;
        for count in counts {
            budget.charge(count, 128)?;
        }
        for c in arrays.into_iter().flatten() {
            for p in &doc.properties[doc.nodes[c].properties.clone()] {
                if let crate::syntax::Property::String(v) = p {
                    budget.charge(v.len(), 8)?;
                }
            }
        }
    }
    Ok(())
}
fn validate_cycles(
    elements: &[crate::canon_scene::Element],
    budget: &Budget,
) -> Result<(), String> {
    let anim = |k: usize| {
        matches!(
            elements[k].ty,
            T::AnimCurve | T::AnimValue | T::AnimLayer | T::AnimStack
        )
    };
    let mut color = vec![0u8; elements.len()];
    for start in 0..elements.len() {
        if !anim(start) || color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack = vec![(start, 0)];
        while let Some((i, next)) = stack.last_mut() {
            let conns = &elements[*i].src_conns;
            if *next == conns.len() {
                color[*i] = 2;
                stack.pop();
                continue;
            }
            let k = conns[*next].0;
            budget.work(1)?;
            *next += 1;
            if !anim(k) {
                continue;
            }
            if color[k] == 1 {
                return Err("animation: connection cycle".into());
            }
            if color[k] == 0 {
                color[k] = 1;
                stack.push((k, 0));
            }
        }
    }
    Ok(())
}
/// Element-indexed helper and owner membership, built once per projection.
pub(crate) fn helper_index(
    n: usize,
    pairs: &[(usize, usize)],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut helper = vec![None; n];
    let mut owner = vec![None; n];
    for &(o, h) in pairs {
        helper[o] = Some(h);
        owner[h] = Some(o);
    }
    (helper, owner)
}
pub(crate) fn project(scn: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    let budget = Budget::new(
        limits.max_allocation,
        limits.max_animation_work,
        Path::Curves,
    );
    budget.charge(scn.doc.nodes.len(), 64)?;
    budget.charge(scn.elements.len(), 1024)?;
    budget.work(scn.doc.nodes.len() + scn.elements.len())?;
    for e in &scn.elements {
        budget.charge(e.src_conns.len(), 128)?;
        for (_, prop) in &e.src_conns {
            budget.charge(prop.len(), 8)?;
        }
    }
    let children = child_index(&scn.doc);
    preflight_curves(&scn.doc, &scn.elements, &children, limits, &budget)?;
    validate_cycles(&scn.elements, &budget)?;
    let mut a = std::mem::take(&mut scn.animation);
    a.ids = scn.elements.iter().map(|e| e.source_id).collect();
    let mut authored = vec![BTreeMap::new(); scn.elements.len()];
    let mut defaults = vec![BTreeMap::new(); scn.elements.len()];
    let (helper_for, owner_for) = helper_index(scn.elements.len(), &scn.scale_helpers);
    let r = Reader {
        s: scn,
        children,
        budget,
    };
    let channel_index: BTreeMap<_, _> = scn
        .deformers
        .channels
        .iter()
        .map(|(k, c)| (*k, c))
        .collect();
    for (i, e) in scn.elements.iter().enumerate() {
        let Some(n) = e.node else {
            continue;
        };
        if scn.doc.nodes[n].name != b"Shape" {
            let layers = node_layers(
                &scn.doc,
                n,
                owner_for[i].is_some(),
                helper_for[i].is_some(),
                limits,
            )?;
            for p in &layers.authored {
                authored[i].insert(p.name.to_vec(), property(p));
            }
            if let Some(list) = layers.defaults {
                for p in &list {
                    defaults[i].insert(p.name.to_vec(), property(p));
                }
            }
        }
        if e.ty == T::BlendChannel {
            let weight = channel_index.get(&i).map_or(0.0, |c| c.weight);
            let percent = if scn.doc.nodes[n].name == b"Shape" {
                let owner = scn.doc.nodes[n].parent.ok_or("animation: shape owner")?;
                let l = scene_property::layers(
                    &scn.doc,
                    owner,
                    limits.max_properties,
                    limits.max_allocation,
                )
                .map_err(|_| "animation: shape property")?;
                l.find(&e.name).map_or(weight * 100.0, |p| p.value_real[0])
            } else if authored[i].is_empty() {
                use crate::canon_dom::{node_data, NodeData, Scalar};
                r.child(n, b"DeformPercent")
                    .and_then(|c| match node_data(&scn.doc, c, scn.ascii) {
                        NodeData::Values(v) => match v.first() {
                            Some(Scalar::Num(_, f)) => Some(*f),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or(100.0)
            } else {
                weight * 100.0
            };
            authored[i]
                .entry(b"DeformPercent".to_vec())
                .or_insert(EvaluatedProperty {
                    property_type: 3,
                    flags: if scn.doc.nodes[n].name == b"Shape" && percent != 0.0 {
                        0x8000
                    } else {
                        0
                    },
                    components: [percent, 0.0, 0.0, 0.0],
                    ..EvaluatedProperty::default()
                });
        }
        if e.ty == T::AnimCurve {
            a.curves
                .push(r.curve(i, n, limits.max_allocation, limits.max_keys)?);
        }
    }
    let mut curve_index = vec![None; scn.elements.len()];
    for (k, c) in a.curves.iter().enumerate() {
        curve_index[c.element] = Some(k);
    }
    for (i, e) in scn.elements.iter().enumerate() {
        if e.ty != T::AnimValue {
            continue;
        }
        let get = |name: &[u8]| authored[i].get(name).or_else(|| defaults[i].get(name));
        let mut v = AnimationValue {
            element: i,
            source_id: e.source_id.unwrap_or(0),
            default_value: [0.0; 3],
            curves: [None; 3],
        };
        for (c, (plain, prefixed)) in [
            (b"X".as_slice(), b"d|X".as_slice()),
            (b"Y", b"d|Y"),
            (b"Z", b"d|Z"),
        ]
        .into_iter()
        .enumerate()
        {
            v.default_value[c] = get(prefixed)
                .or_else(|| get(plain))
                .map_or(0.0, |p| p.components[0]);
        }
        r.budget.sort_work(e.dst_conns.len())?;
        let mut conns = e.dst_conns.clone();
        conns.sort_by(|a, b| a.1.cmp(&b.1));
        for (src, name) in conns {
            if let Some(k) = curve_index[src] {
                let c = match name.as_slice() {
                    b"Y" | b"d|Y" => 1,
                    b"Z" | b"d|Z" => 2,
                    _ => 0,
                };
                v.curves[c] = Some(k);
                if let Some(p) = get(&name) {
                    v.default_value[c] = p.components[0];
                }
            }
        }
        a.values.push(v);
    }
    let mut value_index = vec![None; scn.elements.len()];
    for (k, v) in a.values.iter().enumerate() {
        value_index[v.element] = Some(k);
    }
    r.budget.path.set(Path::Layers);
    for v in &a.values {
        r.budget.work(scn.elements[v.element].src_conns.len())?;
        for (dst, name) in &scn.elements[v.element].src_conns {
            if name.is_empty() {
                continue;
            }
            let p = authored[*dst].entry(name.clone()).or_insert_with(|| {
                let mut p =
                    defaults[*dst]
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| EvaluatedProperty {
                            flags: 0,
                            components: if name == b"Lcl Scaling" {
                                [1.0, 1.0, 1.0, 0.0]
                            } else {
                                [0.0; 4]
                            },
                            property_type: match name.as_slice() {
                                b"Lcl Translation" => 9,
                                b"Lcl Rotation" => 10,
                                b"Lcl Scaling" => 11,
                                _ => 0,
                            },
                            ..EvaluatedProperty::default()
                        });
                p.flags |= 0x1001;
                p.value_str.clear();
                p.value_blob.clear();
                p
            });
            p.flags |= 0x2000;
        }
    }
    for (i, e) in scn.elements.iter().enumerate() {
        if e.ty != T::AnimLayer {
            continue;
        }
        let get = |name: &[u8]| authored[i].get(name).or_else(|| defaults[i].get(name));
        let int = |name: &[u8]| get(name).map_or(0, |p| p.value_int);
        let mode = int(b"BlendMode");
        let mut layer = AnimationLayer {
            element: i,
            source_id: e.source_id.unwrap_or(0),
            name: String::from_utf8_lossy(&e.name).into_owned(),
            weight: clamp_weight(get(b"Weight").map_or(1.0, |p| p.components[0] / 100.0)),
            weight_is_animated: false,
            blended: mode == 0 || mode == 2,
            additive: mode == 0,
            compose_rotation: int(b"RotationAccumulationMode") == 0,
            compose_scale: int(b"ScaleAccumulationMode") == 0,
            values: Vec::new(),
            bindings: Vec::new(),
        };
        r.budget.sort_work(e.dst_conns.len())?;
        // Precharge the dedup set and value list before the first insertion.
        r.budget.charge(e.dst_conns.len(), 80)?;
        let mut seen = BTreeSet::new();
        for (src, name) in &e.dst_conns {
            if !name.is_empty() {
                continue;
            }
            let Some(value) = value_index[*src] else {
                continue;
            };
            if !seen.insert(value) {
                continue;
            }
            layer.values.push(value);
            r.budget.work(scn.elements[*src].src_conns.len())?;
            for (dst, prop) in &scn.elements[*src].src_conns {
                if !prop.is_empty() {
                    r.budget
                        .charge(1, std::mem::size_of::<AnimationBinding>() * 4)?;
                    r.budget.charge(prop.len(), 2)?;
                    layer.bindings.push(AnimationBinding {
                        element: *dst,
                        source_id: scn.elements[*dst].source_id.unwrap_or(0),
                        property: prop.clone(),
                        value,
                    });
                }
            }
        }
        r.budget.sort_work(layer.bindings.len())?;
        layer
            .bindings
            .sort_by(|a, b| a.element.cmp(&b.element).then(a.property.cmp(&b.property)));
        layer.weight_is_animated = get(b"Weight").is_some_and(|p| p.flags & 0x2000 != 0);
        a.layers.push(layer);
    }
    let mut layer_index = vec![None; scn.elements.len()];
    for (k, l) in a.layers.iter().enumerate() {
        layer_index[l.element] = Some(k);
    }
    for (i, e) in scn.elements.iter().enumerate() {
        if e.ty != T::AnimStack {
            continue;
        }
        let get = |name: &[u8]| authored[i].get(name).or_else(|| defaults[i].get(name));
        let bounds = get(b"LocalStart")
            .zip(get(b"LocalStop"))
            .or_else(|| get(b"ReferenceStart").zip(get(b"ReferenceStop")));
        let (time_begin, time_end) = bounds.map_or((0.0, 0.0), |(b, e)| {
            (b.value_int as f64 / KTIME, e.value_int as f64 / KTIME)
        });
        r.budget.sort_work(e.dst_conns.len())?;
        r.budget.charge(e.dst_conns.len(), 80)?;
        let mut seen = BTreeSet::new();
        let mut layers = Vec::new();
        for (src, name) in &e.dst_conns {
            if !name.is_empty() {
                continue;
            }
            if let Some(k) = layer_index[*src] {
                if seen.insert(k) {
                    layers.push(k);
                }
            }
        }
        a.stacks.push(AnimationStack {
            element: i,
            source_id: e.source_id.unwrap_or(0),
            name: String::from_utf8_lossy(&e.name).into_owned(),
            time_begin,
            time_end,
            layers,
        });
    }
    a.default_stack =
        (0..a.stacks.len()).min_by_key(|&i| scn.elements[a.stacks[i].element].ordinal);
    for (i, (base, def)) in authored.iter().zip(&defaults).enumerate() {
        for (name, p) in def.iter().chain(base) {
            a.properties.insert((i, name.clone()), p.clone());
        }
    }
    let mut node_element = vec![None; scn.doc.nodes.len()];
    for (i, e) in scn.elements.iter().enumerate() {
        if let Some(n) = e.node {
            node_element[n].get_or_insert(i);
        }
    }
    let mut node_slot = vec![None; scn.elements.len()];
    for (k, n) in a.nodes.iter().enumerate() {
        node_slot[n.element] = Some(k);
    }
    // Property-to-property links (including generated legacy shape channels).
    for (i, e) in scn.elements.iter().enumerate() {
        if e.ty == T::BlendChannel && e.node.is_some_and(|n| scn.doc.nodes[n].name == b"Shape") {
            let owner = scn.doc.nodes[e.node.unwrap()].parent;
            if let Some(owner) = owner.and_then(|n| node_element[n]) {
                if a.properties.contains_key(&(owner, e.name.clone())) {
                    a.connections
                        .insert((i, b"DeformPercent".to_vec()), (owner, e.name.clone()));
                    if let Some(p) = a.properties.get_mut(&(i, b"DeformPercent".to_vec())) {
                        p.flags |= 0x8000;
                    }
                    if let Some(p) = authored[i].get_mut(b"DeformPercent".as_slice()) {
                        p.flags |= 0x8000;
                    }
                }
            }
        }
    }
    a.channels = scn.deformers.channels.iter().map(|(i, _)| *i).collect();
    a.clusters = scn
        .deformers
        .clusters
        .iter()
        .map(|(_, c)| {
            (
                c.bone_node.and_then(|e| node_slot[e]),
                if c.bone_node.is_some() {
                    c.geometry_to_bone.0
                } else {
                    c.geometry_to_world.0
                },
            )
        })
        .collect();
    if !scn.lean {
        for (i, e) in scn.elements.iter_mut().enumerate() {
            if let Some(V::Obj(header)) = e.obj.get_mut("element") {
                if let Some(V::Obj(props)) = header.get_mut("props") {
                    let num = authored[i]
                        .values()
                        .filter(|p| p.flags & 0xa000 != 0)
                        .count();
                    if num > 0 {
                        props.set("num_animated", V::U(num as u64));
                        props.set(
                            "props",
                            V::Obj(Obj::new().with(
                                "data",
                                V::List(authored[i].iter().map(|(n, p)| prop_obj(n, p)).collect()),
                            )),
                        );
                    }
                }
            }
        }
        for c in &a.curves {
            let o = &mut scn.elements[c.element].obj;
            o.set(
                "keyframes",
                V::List(
                    c.keys
                        .iter()
                        .map(|k| {
                            V::Obj(
                                Obj::new()
                                    .with("time", V::F(k.time))
                                    .with("value", V::F(k.value))
                                    .with("interpolation", V::I(k.interpolation as i64))
                                    .with(
                                        "left",
                                        V::Obj(
                                            Obj::new()
                                                .with("dx", V::F32(k.left.dx))
                                                .with("dy", V::F32(k.left.dy)),
                                        ),
                                    )
                                    .with(
                                        "right",
                                        V::Obj(
                                            Obj::new()
                                                .with("dx", V::F32(k.right.dx))
                                                .with("dy", V::F32(k.right.dy)),
                                        ),
                                    ),
                            )
                        })
                        .collect(),
                ),
            );
            for (name, v) in [
                ("min_time", c.keys.first().map_or(0.0, |k| k.time)),
                ("max_time", c.keys.last().map_or(0.0, |k| k.time)),
                (
                    "min_value",
                    c.keys
                        .iter()
                        .map(|k| k.value)
                        .reduce(|a, b| if a < b { a } else { b })
                        .unwrap_or(0.0),
                ),
                (
                    "max_value",
                    c.keys
                        .iter()
                        .map(|k| k.value)
                        .reduce(|a, b| if a > b { a } else { b })
                        .unwrap_or(0.0),
                ),
            ] {
                o.set(name, V::F(v));
            }
            for (name, e) in [
                ("pre_extrapolation", c.pre_extrapolation),
                ("post_extrapolation", c.post_extrapolation),
            ] {
                o.set(
                    name,
                    V::Obj(
                        Obj::new()
                            .with("mode", V::I(e.mode as i64))
                            .with("repeat_count", V::I(i64::from(e.repeat_count))),
                    ),
                );
            }
        }
        for v in &a.values {
            let o = &mut scn.elements[v.element].obj;
            o.set("default_value", V::Vec(v.default_value.to_vec()));
            o.set(
                "curves",
                V::List(
                    v.curves
                        .iter()
                        .map(|v| V::Ref(v.map(|k| a.curves[k].element)))
                        .collect(),
                ),
            );
        }
        for l in &a.layers {
            let o = &mut scn.elements[l.element].obj;
            o.set("weight", V::F(l.weight));
            o.set("weight_is_animated", V::Bool(l.weight_is_animated));
            o.set(
                "anim_props",
                V::List(
                    l.bindings
                        .iter()
                        .map(|b| V::Obj(binding_obj(b, &a)))
                        .collect(),
                ),
            );
        }
    }
    scn.animation = a;
    scn.unsupported.retain(|(k, _)| {
        !matches!(
            k,
            crate::scene::UnsupportedKind::AnimationStack
                | crate::scene::UnsupportedKind::AnimationLayer
                | crate::scene::UnsupportedKind::AnimationCurveNode
                | crate::scene::UnsupportedKind::AnimationCurve
        )
    });
    Ok(())
}
/// ufbxi_pre_finalize_scene / ufbxi_setup_scale_helper under COMPENSATE.
pub(crate) fn scale_helpers(
    doc: &crate::syntax::Document<'_>,
    elements: &mut Vec<crate::canon_scene::Element>,
    connections: &mut Vec<(usize, usize, Vec<u8>)>,
    ascii: bool,
    limits: &DumpLimits,
) -> Result<Vec<(usize, usize)>, String> {
    let budget = Budget::new(
        limits.max_allocation,
        limits.max_animation_work,
        Path::Helpers,
    );
    budget.charge(doc.nodes.len(), 64)?;
    budget.charge(elements.len(), 512)?;
    budget.work(doc.nodes.len() + elements.len())?;
    let dom_children = child_index(doc);
    preflight_curves(doc, elements, &dom_children, limits, &budget)?;
    let n = elements.len();
    let mut original = vec![0; n];
    let mut scale = vec![[1.0; 3]; n];
    let mut animated = vec![false; n];
    let mut parent = vec![None; n];
    for (i, e) in elements.iter().enumerate() {
        if e.ty != T::Node {
            continue;
        }
        let Some(node) = e.node else {
            continue;
        };
        let l = scene_property::layers(doc, node, limits.max_properties, limits.max_allocation)
            .map_err(|_| "animation: helper properties")?;
        original[i] = match l.find(b"InheritType").map_or(-1, |p| p.value_int) {
            0 => 2,
            2 => 1,
            _ => 0,
        };
        if let Some(p) = l.find(b"Lcl Scaling") {
            scale[i].copy_from_slice(&p.value_real[..3]);
        }
    }
    let mut constant = vec![[f64::NAN; 3]; n];
    let mut has_constant = vec![true; n];
    for (i, e) in elements.iter().enumerate() {
        if e.ty == T::AnimValue {
            if let Some(node) = e.node {
                let l =
                    scene_property::layers(doc, node, limits.max_properties, limits.max_allocation)
                        .map_err(|_| "animation: constant value")?;
                for (c, (plain, prefixed)) in [
                    (b"X".as_slice(), b"d|X".as_slice()),
                    (b"Y", b"d|Y"),
                    (b"Z", b"d|Z"),
                ]
                .into_iter()
                .enumerate()
                {
                    constant[i][c] = l
                        .find(prefixed)
                        .or_else(|| l.find(plain))
                        .map_or(f64::NAN, |p| p.value_real[0]);
                }
            }
        }
    }
    let mut ranges = vec![None; n];
    for (i, e) in elements.iter().enumerate() {
        if e.ty != T::AnimCurve {
            continue;
        }
        let node = e.node.ok_or("animation: curve source")?;
        let value_node = dom_children[node]
            .iter()
            .copied()
            .find(|&c| doc.nodes[c].name == b"KeyValueFloat")
            .ok_or("animation: missing curve array")?;
        budget.work(crate::canon_dom::data_len(doc, value_node)?)?;
        let values = crate::canon_dom::node_data(doc, value_node, ascii);
        ranges[i] = Some(match values {
            crate::canon_dom::NodeData::Reals(v) => v
                .into_iter()
                .fold(None::<(f64, f64)>, |acc, x| {
                    Some(acc.map_or((x, x), |(a, b)| {
                        (if a < x { a } else { x }, if b > x { b } else { x })
                    }))
                })
                .unwrap_or((0.0, 0.0)),
            _ => (0.0, 0.0),
        });
    }
    budget.work(connections.len())?;
    for (src, dst, prop) in connections.iter() {
        if elements[*src].ty == T::AnimCurve && elements[*dst].ty == T::AnimValue {
            let (min, max) = ranges[*src].ok_or("animation: curve range")?;
            let c = match prop.as_slice() {
                b"Y" | b"d|Y" => 1,
                b"Z" | b"d|Z" => 2,
                _ => 0,
            };
            if max - min >= f64::from(0.001f32) {
                has_constant[*dst] = false;
            } else {
                let value = (min + max) * 0.5;
                if constant[*dst][c].is_nan() {
                    constant[*dst][c] = value;
                }
                if (constant[*dst][c] - value).abs() > f64::from(0.001f32) {
                    has_constant[*dst] = false;
                }
            }
        }
    }
    budget.work(connections.len())?;
    for (src, dst, prop) in connections.iter() {
        if elements[*src].ty == T::AnimValue && prop == b"Lcl Scaling" {
            animated[*dst] |= !has_constant[*src]
                || (constant[*src][0] - scale[*dst][0]).abs()
                    + (constant[*src][1] - scale[*dst][1]).abs()
                    + (constant[*src][2] - scale[*dst][2]).abs()
                    >= f64::from(0.001f32);
        }
        if elements[*src].ty == T::Node
            && elements[*dst].ty == T::Node
            && prop.is_empty()
            && parent[*src].is_none()
        {
            parent[*src] = Some(*dst);
        }
    }
    let mut required = vec![false; n];
    for (i, p) in parent.iter().enumerate() {
        if original[i] != 0 {
            if let Some(p) = p.filter(|&p| p != 0) {
                let v = scale[p];
                let r = v[0];
                if animated[p]
                    || (v[0] - r).abs() + (v[1] - r).abs() + (v[2] - r).abs() >= f64::from(0.001f32)
                    || v[0].abs() <= f64::from(0.01f32)
                {
                    required[p] = true;
                }
            }
        }
    }
    // ufbxi_pre_finalize_scene recursively carries helpers through a
    // componentwise branch, including ignore-parent-scale descendants.
    let mut children = vec![Vec::new(); n];
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = p {
            children[*p].push(i);
        }
    }
    let initial: Vec<_> = (1..n).filter(|&i| required[i]).collect();
    let mut recursive = vec![false; n];
    for owner in initial {
        let mut pending = children[owner].clone();
        while let Some(i) = pending.pop() {
            budget.work(1)?;
            if (parent[i] != Some(owner) || original[i] == 2) && original[i] != 0 && !recursive[i] {
                recursive[i] = true;
                required[i] = true;
                pending.extend_from_slice(&children[i]);
            }
        }
    }
    let mut pairs = Vec::new();
    let mut helpers = vec![None; n];
    for i in 1..n {
        if required[i] {
            if elements.len() >= limits.index.max_objects {
                return Err("animation: helper object limit".into());
            }
            budget.charge(1, 1024)?;
            let e = &elements[i];
            let helper = elements.len();
            elements.push(crate::canon_scene::Element {
                key: format!("synthetic:{}:role:scale-helper:type:1:ordinal:0", e.key),
                ordinal: e.ordinal,
                source_id: e.source_id,
                ty: T::Node,
                node: e.node,
                name: Vec::new(),
                src_conns: Vec::new(),
                dst_conns: Vec::new(),
                instances: Vec::new(),
                obj: Obj::new(),
            });
            helpers[i] = Some(helper);
            pairs.push((i, helper));
        }
    }
    budget.work(connections.len())?;
    for (src, dst, prop) in connections.iter_mut() {
        if let Some(h) = helpers.get(*dst).copied().flatten() {
            let redirect = match elements[*src].ty {
                T::Node => original[*src] == 0,
                T::AnimValue => prop == b"Lcl Scaling",
                _ => true,
            };
            if redirect {
                *dst = h;
            }
        } else if elements[*dst].ty == T::SkinCluster {
            if let Some(h) = helpers.get(*src).copied().flatten() {
                *src = h;
            }
        }
    }
    for &(owner, helper) in &pairs {
        connections.push((helper, owner, Vec::new()));
    }
    Ok(pairs)
}
pub(crate) fn node_layers<'a>(
    doc: &'a crate::syntax::Document<'a>,
    n: usize,
    helper: bool,
    owner: bool,
    limits: &DumpLimits,
) -> Result<scene_property::PropertyLayers<'a>, String> {
    let mut l = scene_property::layers(doc, n, limits.max_properties, limits.max_allocation)
        .map_err(|e| format!("node properties: {e:?}"))?;
    let moved = |name: &[u8]| {
        matches!(
            name,
            b"Lcl Scaling" | b"GeometricTranslation" | b"GeometricRotation" | b"GeometricScaling"
        )
    };
    if helper {
        l.authored.retain(|p| moved(p.name));
        l.defaults = None;
    }
    if owner {
        for p in &mut l.authored {
            if moved(p.name) {
                p.value_real = [0.0; 4];
                if p.name == b"Lcl Scaling" || p.name == b"GeometricScaling" {
                    p.value_real[..3].fill(1.0);
                }
                p.value_int = crate::numeric::saturating_i64(p.value_real[0]);
            }
        }
    }
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Property;

    const INPUT: &[u8] = br#"; FBX 7.4.0
Objects: {
 AnimationCurve: 1, "AnimCurve::C", "" {
 KeyTime: *2 { a: 0,46186158000 }
 KeyValueFloat: *2 { a: 0,10 }
 KeyAttrFlags: *1 { a: 4 }
 KeyAttrDataFloat: *4 { a: 0,0,0,0 }
 KeyAttrRefCount: *1 { a: 2 }
 }
 AnimationCurveNode: 2, "AnimCurveNode::T", "" { }
 AnimationLayer: 3, "AnimLayer::L", "" { }
 AnimationStack: 4, "AnimStack::S", "" { }
 Model: 5, "Model::N", "Null" { }
}
Connections: {
 C: "OP",1,2,"d|X"
 C: "OO",2,3
 C: "OO",3,4
 C: "OP",2,5,"Lcl Translation"
}
"#;
    #[test]
    fn every_curve_array_is_preflighted_before_helper_decoding() {
        for name in [
            b"KeyTime".as_slice(),
            b"KeyValueFloat",
            b"KeyAttrFlags",
            b"KeyAttrDataFloat",
            b"KeyAttrRefCount",
        ] {
            let limits = DumpLimits::default();
            let mut s = crate::canon_scene::project_with(INPUT, &limits, true).unwrap();
            let c = s.doc.nodes.iter().position(|n| n.name == name).unwrap();
            let p = s.doc.nodes[c].properties.start;
            // Impossible binary payload must be rejected using its declared count,
            // before node_data can discard the bytes as empty or allocate them.
            s.doc.properties[p] = Property::Array {
                kind: b'd',
                count: usize::MAX,
                big_endian: false,
                bytes: Vec::new().into(),
            };
            let mut connections = Vec::new();
            let error = scale_helpers(&s.doc, &mut s.elements, &mut connections, false, &limits)
                .unwrap_err();
            assert!(
                error.contains("key limit")
                    || error.contains("mismatched")
                    || error.contains("allocation"),
                "{name:?}: {error}"
            );
        }
        let s = crate::canon_scene::project_with(INPUT, &DumpLimits::default(), true).unwrap();
        let children = child_index(&s.doc);
        let budget = Budget::new(0, usize::MAX, Path::Curves);
        assert!(preflight_curves(
            &s.doc,
            &s.elements,
            &children,
            &DumpLimits::default(),
            &budget
        )
        .unwrap_err()
        .contains("allocation limit"));
        let mut elements = s.elements.clone();
        let curve = elements
            .iter()
            .find(|e| e.ty == T::AnimCurve)
            .unwrap()
            .clone();
        // Each curve fits alone, but their combined reconstruction does not.
        elements.push(curve);
        let budget = Budget::new(2500, usize::MAX, Path::Curves);
        assert!(preflight_curves(
            &s.doc,
            &elements,
            &children,
            &DumpLimits::default(),
            &budget
        )
        .unwrap_err()
        .contains("allocation limit"));
    }

    #[test]
    fn multiplicative_bindings_are_charged_before_copying() {
        let mut s = crate::canon_scene::project_with(INPUT, &DumpLimits::default(), true).unwrap();
        let value = s
            .elements
            .iter()
            .position(|e| e.ty == T::AnimValue)
            .unwrap();
        let target = s
            .elements
            .iter()
            .position(|e| e.ty == T::Node && e.node.is_some())
            .unwrap();
        let layer = s
            .elements
            .iter()
            .position(|e| e.ty == T::AnimLayer)
            .unwrap();
        for _ in 0..32 {
            let mut e = s.elements[layer].clone();
            e.dst_conns = vec![(value, Vec::new())];
            s.elements.push(e);
        }
        s.elements[value].src_conns = (0..128)
            .map(|k| (target, format!("Property{k}").into_bytes()))
            .collect();
        let limits = DumpLimits {
            max_allocation: 100_000,
            ..Default::default()
        };
        assert!(project(&mut s, &limits)
            .unwrap_err()
            .contains("allocation limit"));
    }
}
