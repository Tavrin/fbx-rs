//! Curve reconstruction and property bindings from pinned ufbx 0.23.1 (MIT).
use crate::animation::*;
use crate::canon::{s, Obj, V};
use crate::canon_scene::{CanonScene, DumpLimits, ElementType as T};
use crate::scene_property;
use std::collections::BTreeMap;

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
    slope as f32
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
    slope as f32
}
struct Reader<'s, 'd> {
    s: &'s CanonScene<'d>,
    children: Vec<Vec<usize>>,
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
    fn ext(&self, n: usize, name: &[u8]) -> Extrapolation {
        use crate::canon_dom::{node_data, NodeData, Scalar};
        let mut out = Extrapolation {
            mode: ExtrapolationMode::Constant,
            repeat_count: -1,
        };
        let Some(n) = self.child(n, name) else {
            return out;
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
            out.mode = match c as u8 {
                b'A' => ExtrapolationMode::RepeatRelative,
                b'K' => ExtrapolationMode::Slope,
                b'M' => ExtrapolationMode::Mirror,
                b'R' => ExtrapolationMode::Repeat,
                _ => ExtrapolationMode::Constant,
            };
            out.repeat_count = number(b"Repetition").unwrap_or(-1).max(-1) as i32;
        }
        out
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
                .checked_add(r as usize)
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
                    sl = (d00 * ls + d01 * rs) as f32;
                    sr = (d10 * ls + d11 * rs) as f32;
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
                    ((values[k + 1] - value) / (next - time)) as f32
                } else {
                    0.0
                };
                nsl = sr;
            }
            let ld = if time > prev {
                (f64::from(wl) * (time - prev)) as f32
            } else {
                0.0
            };
            let rd = if next > time {
                (f64::from(wr) * (next - time)) as f32
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
            pre_extrapolation: self.ext(n, b"Pre-Extrapolation"),
            post_extrapolation: self.ext(n, b"Post-Extrapolation"),
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
pub(crate) fn project(scn: &mut CanonScene<'_>, limits: &DumpLimits) -> Result<(), String> {
    let mut children = vec![Vec::new(); scn.doc.nodes.len()];
    for (i, n) in scn.doc.nodes.iter().enumerate() {
        if let Some(p) = n.parent {
            children[p].push(i);
        }
    }
    let mut a = std::mem::take(&mut scn.animation);
    a.ids = scn.elements.iter().map(|e| e.source_id).collect();
    let mut retained_keys = 0usize;
    let mut authored = vec![BTreeMap::new(); scn.elements.len()];
    let mut defaults = vec![BTreeMap::new(); scn.elements.len()];
    let r = Reader { s: scn, children };
    for (i, e) in scn.elements.iter().enumerate() {
        let Some(n) = e.node else {
            continue;
        };
        if scn.doc.nodes[n].name != b"Shape" {
            let layers = node_layers(
                &scn.doc,
                n,
                scn.scale_helpers.iter().any(|&(_, h)| h == i),
                scn.scale_helpers.iter().any(|&(o, _)| o == i),
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
            let weight = scn
                .deformers
                .channels
                .iter()
                .find(|(k, _)| *k == i)
                .map_or(0.0, |(_, c)| c.weight);
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
            let tick_node = r
                .child(n, b"KeyTime")
                .ok_or("animation: missing key times")?;
            let raw = &scn.doc.properties[scn.doc.nodes[tick_node].properties.clone()];
            let count = match raw.first() {
                Some(crate::syntax::Property::Array { count, .. }) => *count,
                Some(crate::syntax::Property::AsciiArray(v)) => v.len(),
                _ => raw.len(),
            };
            if count > limits.max_keys {
                return Err("animation: key limit".into());
            }
            retained_keys = retained_keys
                .checked_add(count)
                .ok_or("animation: key count overflow")?;
            // Include keys and the parallel temporary reconstruction arrays.
            if retained_keys
                .checked_mul(128)
                .is_none_or(|b| b > limits.max_allocation)
            {
                return Err("animation: aggregate key allocation limit".into());
            }
            a.curves
                .push(r.curve(i, n, limits.max_allocation, limits.max_keys)?);
        }
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
        let mut conns = e.dst_conns.clone();
        conns.sort_by(|a, b| a.1.cmp(&b.1));
        for (src, name) in conns {
            if let Some(k) = a.curves.iter().position(|v| v.element == src) {
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
    for v in &a.values {
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
        for (src, name) in &e.dst_conns {
            if !name.is_empty() {
                continue;
            }
            let Some(value) = a.values.iter().position(|v| v.element == *src) else {
                continue;
            };
            if layer.values.contains(&value) {
                continue;
            }
            layer.values.push(value);
            for (dst, prop) in &scn.elements[*src].src_conns {
                if !prop.is_empty() {
                    layer.bindings.push(AnimationBinding {
                        element: *dst,
                        source_id: scn.elements[*dst].source_id.unwrap_or(0),
                        property: prop.clone(),
                        value,
                    });
                }
            }
        }
        layer
            .bindings
            .sort_by(|a, b| a.element.cmp(&b.element).then(a.property.cmp(&b.property)));
        layer.weight_is_animated = get(b"Weight").is_some_and(|p| p.flags & 0x2000 != 0);
        a.layers.push(layer);
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
        let mut layers = Vec::new();
        for (src, name) in &e.dst_conns {
            if !name.is_empty() {
                continue;
            }
            if let Some(k) = a.layers.iter().position(|l| l.element == *src) {
                if !layers.contains(&k) {
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
    // Property-to-property links (including generated legacy shape channels).
    for (i, e) in scn.elements.iter().enumerate() {
        if e.ty == T::BlendChannel && e.node.is_some_and(|n| scn.doc.nodes[n].name == b"Shape") {
            let owner = scn.doc.nodes[e.node.unwrap()].parent;
            if let Some(owner) =
                owner.and_then(|n| scn.elements.iter().position(|e| e.node == Some(n)))
            {
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
                c.bone_node
                    .and_then(|e| a.nodes.iter().position(|n| n.element == e)),
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
    for (src, dst, prop) in connections.iter() {
        if elements[*src].ty == T::AnimCurve && elements[*dst].ty == T::AnimValue {
            let node = elements[*src].node.ok_or("animation: curve source")?;
            let values = doc
                .nodes
                .iter()
                .position(|d| d.parent == Some(node) && d.name == b"KeyValueFloat")
                .map(|i| crate::canon_dom::node_data(doc, i, ascii));
            let (min, max) = match values {
                Some(crate::canon_dom::NodeData::Reals(v)) => v
                    .into_iter()
                    .fold(None::<(f64, f64)>, |acc, x| {
                        Some(acc.map_or((x, x), |(a, b)| {
                            (if a < x { a } else { x }, if b > x { b } else { x })
                        }))
                    })
                    .unwrap_or((0.0, 0.0)),
                _ => (0.0, 0.0),
            };
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
                p.value_int = p.value_real[0] as i64;
            }
        }
    }
    Ok(l)
}
