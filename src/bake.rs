//! Bounded animation baking. Key time generation and reduction follow ufbx
//! 0.23.1. See `docs/VERIFICATION.md` for remaining compatibility boundaries.
use crate::animation::{Animation, BakeSample, NodeInputs, TransformChannel, KTIME};
use crate::api_alloc::Budget;
use crate::{AnimationBinding, Error, Interpolation};
use std::cell::Cell;

/// Controls for bounded baking. Defaults match ufbx's sampling defaults,
/// with reduction enabled and an aggregate 16 million candidate key cap.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BakeOptions {
    /// Shift positive start times to zero using FBX ticks.
    pub trim_start_time: bool,
    /// Samples per second for nonlinear segments.
    pub resample_rate: f64,
    /// Segments sampled at least this frequently are not resampled.
    pub minimum_sample_rate: f64,
    /// Maximum generated subdivisions of one source segment.
    pub max_keyframe_segments: usize,
    /// Reduce each channel independently.
    pub key_reduction_enabled: bool,
    /// Use spherical interpolation when reducing rotations.
    pub key_reduction_rotation: bool,
    /// Linear reduction error threshold; negative disables reduction.
    pub key_reduction_threshold: f64,
    /// Maximum reduction passes.
    pub key_reduction_passes: usize,
    /// Bake raw transform properties as well as node transforms.
    pub bake_transform_properties: bool,
    /// Aggregate cap including candidate keys before reduction and deduplication.
    pub max_keys: usize,
    /// Aggregate requested vector capacity for this bake (bytes).
    pub max_allocation: usize,
    /// Maximum evaluation work: static input lookups, node/dependency visits,
    /// and property evaluations and their layer/connection/animated-weight visits.
    pub max_evaluation_work: usize,
}
impl Default for BakeOptions {
    fn default() -> Self {
        Self {
            trim_start_time: false,
            resample_rate: 30.0,
            minimum_sample_rate: 19.5,
            max_keyframe_segments: 32,
            key_reduction_enabled: true,
            key_reduction_rotation: false,
            key_reduction_threshold: 0.000001,
            key_reduction_passes: 4,
            bake_transform_properties: false,
            max_keys: 16_000_000,
            max_allocation: 768 << 20,
            max_evaluation_work: 67_108_864,
        }
    }
}
/// A baked linear vector channel key. Flags use ufbx's baked-key bit values.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BakedKey<const N: usize> {
    /// Time in seconds, shifted when trimming is enabled.
    pub time: f64,
    /// Vector or quaternion value.
    pub value: [f64; N],
    /// Step-left 1, step-right 2, step-key 4, source-key 8.
    pub flags: u32,
}
/// Independently reduced transform channels for an animated node.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BakedNode {
    /// Index into [`crate::Scene::nodes`].
    pub node: usize,
    /// True if every translation value is equal.
    pub constant_translation: bool,
    /// True if every quaternion is equal after antipodal correction.
    pub constant_rotation: bool,
    /// True if every scale value is equal.
    pub constant_scale: bool,
    /// Translation keys.
    pub translation_keys: Vec<BakedKey<3>>,
    /// Quaternion keys (x, y, z, w).
    pub rotation_keys: Vec<BakedKey<4>>,
    /// Scale keys.
    pub scale_keys: Vec<BakedKey<3>>,
}
/// Independently reduced numeric source property channel.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BakedProperty {
    /// Original target object ID.
    pub source_id: i64,
    /// Byte-exact property name.
    pub name: Vec<u8>,
    /// True if every value is equal.
    pub constant_value: bool,
    /// Vector keys (scalar properties use the first component).
    pub keys: Vec<BakedKey<3>>,
}
/// Baked animation with the stack playback range and key range.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BakedAnimation {
    /// Animated nodes, in public node order.
    pub nodes: Vec<BakedNode>,
    /// Animated properties, in target/property order.
    pub properties: Vec<BakedProperty>,
    /// Original stack start, retained even when key times are trimmed.
    pub playback_time_begin: f64,
    /// Original stack end, retained even when key times are trimmed.
    pub playback_time_end: f64,
    /// Original stack duration.
    pub playback_duration: f64,
    /// Earliest key time before optional trimming, as in ufbx 0.23.1.
    pub key_time_min: f64,
    /// Latest key time before optional trimming.
    pub key_time_max: f64,
}
#[derive(Clone, Copy)]
struct Time {
    time: f64,
    flags: u32,
}
struct Context<'a> {
    a: &'a Animation,
    opts: &'a BakeOptions,
    budget: Budget,
    keys: Cell<usize>,
    work: Cell<usize>,
    layers: &'a [usize],
    begin: f64,
    end: f64,
    offset: f64,
    min: Cell<f64>,
    max: Cell<f64>,
}
impl Context<'_> {
    fn charge(&self) -> Result<(), Error> {
        let count = self
            .keys
            .get()
            .checked_add(1)
            .ok_or(Error::ResourceLimit("bake keys"))?;
        if count > self.opts.max_keys {
            return Err(Error::ResourceLimit("bake keys"));
        }
        self.keys.set(count);
        Ok(())
    }
    fn push_time(&self, out: &mut Vec<Time>, time: f64, flags: u32) -> Result<(), Error> {
        if !time.is_finite() {
            return Err(Error::ResourceLimit("bake time"));
        }
        self.charge()?;
        self.budget.push(out, Time { time, flags })
    }
    fn work(&self, n: usize) -> Result<(), Error> {
        let work = self
            .work
            .get()
            .checked_add(n)
            .ok_or(Error::ResourceLimit("bake evaluation work"))?;
        if work > self.opts.max_evaluation_work {
            return Err(Error::ResourceLimit("bake evaluation work"));
        }
        #[cfg(test)]
        EVAL_WORK.with(|w| w.set(w.get() + n));
        self.work.set(work);
        Ok(())
    }
    // `ufbxi_bake_times`: component keys and nonlinear segment samples.
    fn curve_times(
        &self,
        out: &mut Vec<Time>,
        binding: &AnimationBinding,
        linear: bool,
        flags: u32,
    ) -> Result<(), Error> {
        let v = &self.a.values[binding.value];
        for curve in v.curves.iter().flatten().map(|&i| &self.a.curves[i]) {
            for (i, a) in curve.keys.iter().enumerate() {
                self.push_time(out, a.time, flags)?;
                let Some(b) = curve.keys.get(i + 1) else {
                    break;
                };
                if a.value == b.value && a.right.dy == 0.0 && b.left.dy == 0.0 {
                    continue;
                }
                match a.interpolation {
                    Interpolation::ConstantPrevious => self.push_time(out, b.time, 1)?,
                    Interpolation::ConstantNext => self.push_time(out, a.time, 2)?,
                    _ if linear || a.interpolation == Interpolation::Cubic => {
                        let duration = b.time - a.time;
                        if duration <= 1.0 / self.opts.minimum_sample_rate {
                            continue;
                        }
                        let rate = self.opts.resample_rate;
                        let mut factor = 1.0;
                        while duration * rate / factor >= self.opts.max_keyframe_segments as f64 {
                            factor *= 2.0;
                        }
                        let pad = 0.5 / rate;
                        let start = ((a.time + pad) * rate / factor).ceil() * factor;
                        for j in 0..self.opts.max_keyframe_segments {
                            let time = (start + j as f64 * factor) / rate;
                            if time >= b.time - pad {
                                break;
                            }
                            self.push_time(out, time, 0)?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    // `ufbxi_finalize_bake_times`, excluding maximum-rate culling.
    fn finalize(&self, mut times: Vec<Time>, weight_times: &[Time]) -> Result<Vec<Time>, Error> {
        for &t in weight_times {
            self.push_time(&mut times, t.time, t.flags)?;
        }
        if times.is_empty() {
            self.push_time(&mut times, self.begin, 0)?;
            self.push_time(&mut times, self.end, 0)?;
        }
        times.sort_unstable_by(|a, b| {
            a.time
                .total_cmp(&b.time)
                .then(((a.flags & 3) ^ 1).cmp(&((b.flags & 3) ^ 1)))
        });
        let mut dst = 0;
        for i in 0..times.len() {
            let mut t = times[i];
            if dst > 0 && times[dst - 1].time == t.time {
                if (times[dst - 1].flags ^ t.flags) & 3 == 0 {
                    times[dst - 1].flags |= t.flags;
                    continue;
                }
                if times[dst - 1].flags & 1 != 0 {
                    t.flags |= 4;
                } else if t.flags & 2 != 0 {
                    times[dst - 1].flags |= 4;
                }
            }
            times[dst] = t;
            dst += 1;
        }
        times.truncate(dst);
        dst = 0;
        for i in 0..times.len() {
            let t = times[i];
            let before = if dst > 0 {
                t.time - times[dst - 1].time
            } else {
                f64::INFINITY
            };
            let after = times.get(i + 1).map_or(f64::INFINITY, |n| n.time - t.time);
            if t.flags & 15 == 0 && before.min(after) < 0.25 / self.opts.resample_rate {
                continue;
            }
            times[dst] = t;
            dst += 1;
        }
        times.truncate(dst);
        if let (Some(first), Some(last)) = (times.first(), times.last()) {
            self.min.set(self.min.get().min(first.time));
            self.max.set(self.max.get().max(last.time));
        }
        Ok(times)
    }
    fn sample(&self, t: Time) -> f64 {
        if t.flags & 3 == 0 {
            t.time
        } else {
            next_after(t.time, t.flags & 1 == 0)
        }
    }
    // `ufbxi_bake_postprocess_vec3`/`quat` and default `ufbxi_postprocess_step`.
    fn post<const N: usize>(&self, mut keys: Vec<BakedKey<N>>) -> (Vec<BakedKey<N>>, bool) {
        if self.offset != 0.0 {
            for k in &mut keys {
                k.time = (k.time * KTIME + self.offset).round_ties_even() / KTIME;
            }
        }
        let mut dst = 0;
        let mut prev = keys.first().map_or(0.0, |k| k.time);
        for i in 0..keys.len() {
            let mut k = keys[i];
            if k.flags & 3 != 0 {
                let next = keys.get(i + 1).map_or(f64::INFINITY, |n| n.time);
                let eps = 1.0 + f64::from(f32::EPSILON * 4.0);
                if k.flags & 1 != 0 {
                    let min = (prev + 0.001).max(if prev > 0.0 { prev * eps } else { prev / eps });
                    k.time = (k.time - 0.001).min(if k.time > 0.0 {
                        k.time / eps
                    } else {
                        k.time * eps
                    });
                    if k.time <= min {
                        continue;
                    }
                } else {
                    let max = (next - 0.001).min(if next > 0.0 { next / eps } else { next * eps });
                    k.time = (k.time + 0.001).max(if k.time > 0.0 {
                        k.time * eps
                    } else {
                        k.time / eps
                    });
                    if k.time >= max {
                        continue;
                    }
                }
                if k.time == prev || k.time == next {
                    continue;
                }
            }
            prev = k.time;
            keys[dst] = k;
            dst += 1;
        }
        keys.truncate(dst);
        if N == 4 {
            for i in 1..keys.len() {
                if dot(&keys[i - 1].value, &keys[i].value) < 0.0 {
                    for v in &mut keys[i].value {
                        *v = -*v;
                    }
                }
            }
        }
        if self.opts.key_reduction_enabled && self.opts.key_reduction_threshold >= 0.0 {
            reduce(&mut keys, self.opts);
        }
        let constant = keys
            .first()
            .is_some_and(|first| keys.iter().all(|k| k.value == first.value));
        (keys, constant)
    }
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
fn dot<const N: usize>(a: &[f64; N], b: &[f64; N]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
// Reduction passes from `ufbxi_bake_postprocess_vec3`/`quat`.
fn reduce<const N: usize>(keys: &mut Vec<BakedKey<N>>, opts: &BakeOptions) {
    for _ in 0..opts.key_reduction_passes {
        if keys.len() < 3 {
            break;
        }
        let mut dst = 1;
        let mut i = 1;
        while i < keys.len() {
            let (prev, cur) = (keys[i - 1], keys[i]);
            if let Some(next) = keys.get(i + 1).copied() {
                let t = (cur.time - prev.time) / (next.time - prev.time);
                let mut error = 0.0;
                if N == 4 && !opts.key_reduction_rotation {
                    for c in 0..N {
                        let d = prev.value[c] - cur.value[c];
                        error += d * d;
                    }
                    for c in 0..N {
                        let d = next.value[c] - cur.value[c];
                        error += d * d;
                    }
                    error *= 0.5;
                } else if N == 4 {
                    let a = [prev.value[0], prev.value[1], prev.value[2], prev.value[3]];
                    let b = [next.value[0], next.value[1], next.value[2], next.value[3]];
                    let tmp = crate::animation::slerp(a, b, t);
                    for (v, cur) in tmp.iter().zip(&cur.value) {
                        let d = *v - *cur;
                        error += d * d;
                    }
                } else {
                    for c in 0..N {
                        let d = prev.value[c] * (1.0 - t) + next.value[c] * t - cur.value[c];
                        error += d * d;
                    }
                }
                if error <= opts.key_reduction_threshold * opts.key_reduction_threshold {
                    keys[dst] = next;
                    dst += 1;
                    i += 2;
                    continue;
                }
            }
            keys[dst] = cur;
            dst += 1;
            i += 1;
        }
        if dst == keys.len() {
            break;
        }
        keys.truncate(dst);
    }
}
fn transform_prop(p: &[u8]) -> bool {
    matches!(
        p,
        b"Lcl Translation"
            | b"Lcl Rotation"
            | b"Lcl Scaling"
            | b"PreRotation"
            | b"PostRotation"
            | b"RotationOffset"
            | b"ScalingOffset"
            | b"RotationPivot"
            | b"ScalingPivot"
            | b"RotationOrder"
    )
}

/// A baked scale helper: later nodes read its keys, as ufbx reads
/// `baked_nodes[helper]`.
#[derive(Clone)]
struct HelperKeys {
    keys: Vec<BakedKey<3>>,
    constant: bool,
}
// `ufbx_evaluate_baked_vec3`.
fn evaluate_baked(keys: &[BakedKey<3>], time: f64) -> [f64; 3] {
    let at = keys.partition_point(|k| k.time <= time);
    let Some(next) = keys.get(at) else {
        return keys.last().map_or([1.0; 3], |k| k.value);
    };
    if at == 0 {
        return next.value;
    }
    let mut prev = at - 1;
    if prev > 0 && keys[prev].flags & 2 != 0 && keys[prev - 1].time == time {
        prev -= 1;
    }
    let prev = &keys[prev];
    if time == prev.time {
        return prev.value;
    }
    let mut t = (time - prev.time) / (next.time - prev.time);
    if prev.flags & 1 != 0 {
        t = 0.0;
    }
    if next.flags & 2 != 0 {
        t = 1.0;
    }
    let u = 1.0 - t;
    std::array::from_fn(|i| prev.value[i] * u + next.value[i] * t)
}
/// `ufbxi_bake_node` state: nodes are baked in public node order, and baking a
/// scale helper also bakes its siblings (`ufbxi_bake_node_imp`).
struct NodeBaker<'a> {
    c: &'a Context<'a>,
    inputs: &'a [NodeInputs],
    weights: &'a [Time],
    /// Child slots of each node slot, in public node order.
    children: Vec<Vec<usize>>,
    baked: Vec<Option<HelperKeys>>,
    to_bake: Vec<bool>,
    stack: Vec<usize>,
    nodes: Vec<BakedNode>,
}
impl NodeBaker<'_> {
    // `ufbxi_push_resampled_times`.
    fn push_resampled(&self, out: &mut Vec<Time>, keys: &[BakedKey<3>]) -> Result<(), Error> {
        for (i, k) in keys.iter().enumerate() {
            let mut time = k.time;
            if k.flags & 1 != 0 && keys.get(i + 1).is_some_and(|n| n.flags & 4 != 0) {
                time = keys[i + 1].time;
            } else if k.flags & 2 != 0 && i > 0 && keys[i - 1].flags & 4 != 0 {
                time = keys[i - 1].time;
            }
            self.c.push_time(out, time, k.flags & 7)?;
        }
        Ok(())
    }
    fn bake(&mut self, node: usize, group: &[&AnimationBinding]) -> Result<(), Error> {
        let c = self.c;
        let a = c.a;
        let cfg = &a.nodes[node];
        let mut channels: [Vec<Time>; 3] = std::array::from_fn(|_| Vec::new());
        // The parent's scale helper scales this node's translation; a helper
        // node is never scaled by another helper.
        let t_helper = if cfg.is_scale_helper {
            None
        } else {
            cfg.translation_scale
        }
        .and_then(|h| self.baked[h].as_ref());
        let mut resample_translation = false;
        if let Some(h) = t_helper {
            resample_translation = !h.constant;
            self.push_resampled(&mut channels[0], &h.keys)?;
        }
        let s_helper = if cfg.is_scale_helper {
            cfg.inherited_scale
        } else {
            None
        }
        .and_then(|h| self.baked[h].as_ref());
        let mut resample_scale = false;
        if let Some(h) = s_helper {
            resample_scale = !h.constant;
            self.push_resampled(&mut channels[2], &h.keys)?;
        }
        let p = &cfg.props;
        let complex = p.scaling_pivot != [0.0; 3]
            || p.rotation_pivot != [0.0; 3]
            || p.rotation_offset != [0.0; 3]
            || p.scaling_offset != [0.0; 3]
            || group.iter().any(|b| {
                matches!(
                    b.property.as_slice(),
                    b"ScalingPivot" | b"RotationPivot" | b"RotationOffset" | b"ScalingOffset"
                )
            });
        for b in group {
            let name = b.property.as_slice();
            if name == b"Lcl Translation" || complex && transform_prop(name) {
                c.curve_times(
                    &mut channels[0],
                    b,
                    name != b"Lcl Translation" || resample_translation,
                    if name == b"Lcl Translation" { 8 } else { 0 },
                )?;
            }
            if matches!(
                name,
                b"Lcl Rotation" | b"PreRotation" | b"PostRotation" | b"RotationOrder"
            ) {
                c.curve_times(
                    &mut channels[1],
                    b,
                    true,
                    if name == b"Lcl Rotation" { 8 } else { 0 },
                )?;
            }
            if name == b"Lcl Scaling" {
                c.curve_times(&mut channels[2], b, resample_scale, 8)?;
            }
        }
        let [t, r, s] = channels;
        let (t, r, s) = (
            c.finalize(t, self.weights)?,
            c.finalize(r, self.weights)?,
            c.finalize(s, self.weights)?,
        );
        let work = |n: usize| c.work(n);
        let sample = |time: f64, ignore: bool| BakeSample {
            layers: c.layers,
            time,
            work: &work,
            ignore_scale_helper: ignore,
        };
        let mut tk = Vec::new();
        let mut rk = Vec::new();
        let mut sk = Vec::new();
        c.budget.reserve(&mut tk, t.len())?;
        c.budget.reserve(&mut rk, r.len())?;
        c.budget.reserve(&mut sk, s.len())?;
        for time in t {
            let at = c.sample(time);
            let mut transform = a.bake_transform(
                self.inputs,
                node,
                TransformChannel::Translation,
                // A helper node's own translation is never scaled by a helper.
                &sample(at, t_helper.is_some() || cfg.is_scale_helper),
            )?;
            if let Some(h) = t_helper {
                let scale = evaluate_baked(&h.keys, at);
                for (v, s) in transform.translation.iter_mut().zip(scale) {
                    *v *= s;
                }
            }
            tk.push(BakedKey {
                time: time.time,
                flags: time.flags,
                value: transform.translation,
            });
        }
        for time in r {
            let transform = a.bake_transform(
                self.inputs,
                node,
                TransformChannel::Rotation,
                &sample(c.sample(time), false),
            )?;
            rk.push(BakedKey {
                time: time.time,
                flags: time.flags,
                value: transform.rotation,
            });
        }
        for time in s {
            let at = c.sample(time);
            let mut transform = a.bake_transform(
                self.inputs,
                node,
                TransformChannel::Scale,
                &sample(at, s_helper.is_some()),
            )?;
            if let Some(h) = s_helper {
                let scale = evaluate_baked(&h.keys, at);
                for (v, s) in transform.scale.iter_mut().zip(scale) {
                    *v *= s;
                }
            }
            sk.push(BakedKey {
                time: time.time,
                flags: time.flags,
                value: transform.scale,
            });
        }
        let (translation_keys, constant_translation) = c.post(tk);
        let (rotation_keys, constant_rotation) = c.post(rk);
        let (scale_keys, constant_scale) = c.post(sk);
        if cfg.is_scale_helper {
            let mut keys = Vec::new();
            c.budget.reserve(&mut keys, scale_keys.len())?;
            keys.extend_from_slice(&scale_keys);
            self.baked[node] = Some(HelperKeys {
                keys,
                constant: constant_scale,
            });
        }
        c.budget.push(
            &mut self.nodes,
            BakedNode {
                node,
                translation_keys,
                rotation_keys,
                scale_keys,
                constant_translation,
                constant_rotation,
                constant_scale,
            },
        )?;
        if cfg.is_scale_helper {
            if let Some(parent) = cfg.parent {
                for i in 0..self.children[parent].len() {
                    let child = self.children[parent][i];
                    if child == node {
                        continue;
                    }
                    if !self.to_bake[child] {
                        self.to_bake[child] = true;
                        c.budget.push(&mut self.stack, child)?;
                    }
                    let cc = &a.nodes[child];
                    if let (Some(inherit), Some(own)) = (cc.inherit_helper, cc.own_helper) {
                        if self.to_bake[inherit] && !self.to_bake[own] {
                            self.to_bake[own] = true;
                            c.budget.push(&mut self.stack, own)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn bake(
    a: &Animation,
    stack: Option<usize>,
    opts: &BakeOptions,
) -> Result<BakedAnimation, Error> {
    if !opts.resample_rate.is_finite()
        || opts.resample_rate <= 0.0
        || !opts.minimum_sample_rate.is_finite()
        || opts.minimum_sample_rate <= 0.0
        || !opts.key_reduction_threshold.is_finite()
        || opts.max_keyframe_segments == 0
        || opts.max_keyframe_segments > 1_000_000
        || opts.key_reduction_passes > 64
    {
        return Err(Error::ResourceLimit("invalid bake options"));
    }
    let layers = a.stack_layers(stack)?;
    let selected = stack.or(a.default_stack).and_then(|i| a.stacks.get(i));
    let (raw_begin, raw_end) = selected.map_or((0.0, 0.0), |s| (s.time_begin, s.time_end));
    if !raw_begin.is_finite() || !raw_end.is_finite() {
        return Err(Error::ResourceLimit("bake stack range"));
    }
    let (begin, end) = if raw_begin < raw_end {
        (raw_begin, raw_end)
    } else {
        (0.0, 0.0)
    };
    let offset = if opts.trim_start_time && raw_begin > 0.0 {
        -raw_begin * KTIME
    } else {
        0.0
    };
    let c = Context {
        a,
        opts,
        budget: Budget::new(opts.max_allocation),
        keys: Cell::new(0),
        work: Cell::new(0),
        layers,
        begin,
        end,
        offset,
        min: Cell::new(f64::INFINITY),
        max: Cell::new(f64::NEG_INFINITY),
    };
    let inputs = c
        .budget
        .try_collect((0..a.nodes.len()).map(|i| a.node_inputs(i, &|n| c.work(n))))?;
    let mut bindings = Vec::new();
    for &l in layers {
        for b in &a.layers[l].bindings {
            c.budget.push(&mut bindings, b)?;
        }
    }
    // ufbx bakes nodes in public node order, then everything else by creation.
    let mut node_slots = c
        .budget
        .collect(a.nodes.iter().enumerate().map(|(i, n)| (n.element, i)))?;
    node_slots.sort_unstable_by_key(|n| n.0);
    let slot_of = |element: usize| {
        node_slots
            .binary_search_by_key(&element, |n| n.0)
            .ok()
            .map(|i| node_slots[i].1)
    };
    let rank = |element: usize| a.creation.get(element).copied().unwrap_or(element);
    bindings.sort_unstable_by(|x, y| {
        (
            slot_of(x.element).unwrap_or(usize::MAX),
            rank(x.element),
            &x.property,
        )
            .cmp(&(
                slot_of(y.element).unwrap_or(usize::MAX),
                rank(y.element),
                &y.property,
            ))
    });
    let mut weights = Vec::new();
    let mut layer_elements = c.budget.collect(a.layers.iter().map(|l| l.element))?;
    layer_elements.sort_unstable();
    for b in &bindings {
        if b.property == b"Weight" && layer_elements.binary_search(&b.element).is_ok() {
            c.curve_times(&mut weights, b, true, 0)?;
        }
    }
    if !weights.is_empty() {
        weights = c.finalize(weights, &[])?;
    }
    let mut baker = NodeBaker {
        c: &c,
        inputs: &inputs,
        weights: &weights,
        children: c.budget.filled(a.nodes.len(), Vec::new())?,
        baked: c.budget.filled(a.nodes.len(), None)?,
        to_bake: c.budget.filled(a.nodes.len(), false)?,
        stack: Vec::new(),
        nodes: Vec::new(),
    };
    for (slot, n) in a.nodes.iter().enumerate() {
        if let Some(parent) = n.parent {
            c.budget.push(&mut baker.children[parent], slot)?;
        }
    }
    for b in &bindings {
        if let Some(slot) = slot_of(b.element) {
            baker.to_bake[slot] = true;
        }
    }
    let mut properties: Vec<(usize, BakedProperty)> = Vec::new();
    let mut at = 0;
    while at < bindings.len() {
        let element = bindings[at].element;
        let mut end = at + 1;
        while end < bindings.len() && bindings[end].element == element {
            end += 1;
        }
        let group = &bindings[at..end];
        let node = slot_of(element);
        if let Some(node) = node {
            baker.bake(node, group)?;
            // Baking a scale helper pulls in its siblings; keep going until
            // every dependency is baked (`ufbxi_bake_node`).
            while let Some(next) = baker.stack.pop() {
                baker.bake(next, &[])?;
            }
        }
        let mut p_at = 0;
        while p_at < group.len() {
            let first = group[p_at];
            let p_end = group.partition_point(|b| b.property <= first.property);
            if node.is_none() || opts.bake_transform_properties || !transform_prop(&first.property)
            {
                let mut times = Vec::new();
                for b in &group[p_at..p_end] {
                    c.curve_times(&mut times, b, false, 8)?;
                }
                let times = c.finalize(times, &weights)?;
                let mut keys = Vec::new();
                c.budget.reserve(&mut keys, times.len())?;
                for time in times {
                    let p =
                        a.bake_property(layers, element, &first.property, c.sample(time), &|n| {
                            c.work(n)
                        })?;
                    keys.push(BakedKey {
                        time: time.time,
                        flags: time.flags,
                        value: [p.components[0], p.components[1], p.components[2]],
                    });
                }
                let (keys, constant_value) = c.post(keys);
                let name = c.budget.bytes(&first.property)?;
                c.budget.push(
                    &mut properties,
                    (
                        rank(element),
                        BakedProperty {
                            source_id: first.source_id,
                            name,
                            keys,
                            constant_value,
                        },
                    ),
                )?;
            }
            p_at = p_end;
        }
        at = end;
    }
    let mut nodes = std::mem::take(&mut baker.nodes);
    nodes.sort_unstable_by_key(|n| n.node);
    // ufbx sorts baked elements by creation index; ties keep property order.
    properties.sort_by_key(|p| p.0);
    let properties = c.budget.collect(properties.into_iter().map(|(_, p)| p))?;
    let (key_time_min, key_time_max) = if c.min.get() < c.max.get() {
        (c.min.get(), c.max.get())
    } else {
        (0.0, 0.0)
    };
    Ok(BakedAnimation {
        nodes,
        properties,
        playback_time_begin: begin,
        playback_time_end: end,
        playback_duration: end - begin,
        key_time_min,
        key_time_max,
    })
}

#[cfg(test)]
thread_local! { static EVAL_WORK: Cell<usize> = const {Cell::new(0)}; }
#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::*;
    use crate::scene_node as sn;
    fn synthetic(n: usize) -> Animation {
        let mut a = Animation {
            allocation_limit: 768 << 20,
            ..Animation::default()
        };
        a.curves.push(AnimationCurve {
            element: 0,
            source_id: 0,
            keys: vec![Keyframe::new(0, 0.0), Keyframe::new(46_186_158_000, 1.0)],
            pre_extrapolation: Extrapolation::new(ExtrapolationMode::Constant, 0),
            post_extrapolation: Extrapolation::new(ExtrapolationMode::Constant, 0),
        });
        a.values.push(AnimationValue {
            element: 0,
            source_id: 0,
            default_value: [0.0; 3],
            curves: [Some(0), None, None],
        });
        let mut bindings = Vec::new();
        for i in 0..=n {
            a.nodes.push(NodeEval {
                element: i,
                parent: None,
                props: sn::NodeProps::default(),
                adjust: sn::Adjust::default(),
                rotation_order: 0,
                use_rotation_space: false,
                translation_scale: None,
                inherited_scale: None,
                is_scale_helper: false,
                own_helper: None,
                inherit_helper: None,
            });
            if i > 0 {
                bindings.push(AnimationBinding {
                    element: i,
                    source_id: i64::try_from(i).unwrap(),
                    property: b"Lcl Translation".to_vec(),
                    value: 0,
                });
                a.properties
                    .try_insert(
                        (i, b"Lcl Translation".to_vec()),
                        EvaluatedProperty {
                            flags: 0x2000,
                            ..Default::default()
                        },
                        &crate::api_alloc::Budget::new(usize::MAX),
                    )
                    .unwrap();
            }
        }
        a.layers.push(AnimationLayer {
            element: n + 1,
            source_id: 1,
            name: String::new(),
            weight: 1.0,
            weight_is_animated: false,
            blended: false,
            additive: false,
            compose_rotation: false,
            compose_scale: false,
            values: vec![0],
            bindings,
        });
        a.stacks.push(AnimationStack {
            element: n + 2,
            source_id: 2,
            name: String::new(),
            time_begin: 0.0,
            time_end: 1.0,
            layers: vec![0],
        });
        a.default_stack = Some(0);
        a
    }
    #[test]
    fn bake_scaling_counts_actual_evaluations() {
        let mut counts = Vec::new();
        for n in [1000, 8000] {
            let a = synthetic(n);
            EVAL_WORK.with(|w| w.set(0));
            let b = bake(&a, None, &BakeOptions::default()).unwrap();
            assert_eq!(b.nodes.len(), n);
            let work = EVAL_WORK.with(Cell::get);
            assert_eq!(work, 25 * n + 15);
            eprintln!("bake nodes={n} evaluations={work}");
            counts.push(work);
        }
        assert!(counts[1] <= 8 * counts[0]);
    }
    fn animated_property(a: &mut Animation, name: &[u8], value: usize) {
        a.layers[0].bindings.push(AnimationBinding {
            element: 1,
            source_id: 1,
            property: name.to_vec(),
            value,
        });
        a.layers[0]
            .bindings
            .sort_unstable_by(|a, b| (a.element, &a.property).cmp(&(b.element, &b.property)));
        a.properties
            .try_insert(
                (1, name.to_vec()),
                EvaluatedProperty {
                    flags: 0x2000,
                    ..Default::default()
                },
                &Budget::new(usize::MAX),
            )
            .unwrap();
    }
    #[test]
    fn bake_scaling_keys_and_unrelated_properties() {
        let mut counts = Vec::new();
        for n in [128, 1024] {
            let mut a = synthetic(1);
            a.curves[0].keys = (0..n)
                .map(|k| Keyframe::new(i64::try_from(k).unwrap() * 46_186_158, k as f64))
                .collect();
            a.values.push(AnimationValue {
                element: 0,
                source_id: 0,
                default_value: [42.0; 3],
                curves: [None; 3],
            });
            for p in 0..n {
                animated_property(&mut a, format!("Unrelated{p:04}").as_bytes(), 1);
            }
            EVAL_WORK.with(|w| w.set(0));
            let b = bake(&a, None, &BakeOptions::default()).unwrap();
            let work = EVAL_WORK.with(Cell::get);
            assert_eq!(b.properties.len(), n);
            assert!(b
                .properties
                .iter()
                .all(|p| p.keys.iter().all(|k| k.value == [42.0; 3])));
            assert!(work <= 12 * n + 100, "{work}");
            eprintln!("bake keys/properties={n} work={work}");
            // Exact budget succeeds, one unit less fails before doing the work.
            let opts = BakeOptions {
                max_evaluation_work: work,
                ..Default::default()
            };
            assert_eq!(bake(&a, None, &opts).unwrap(), b);
            assert_eq!(
                bake(
                    &a,
                    None,
                    &BakeOptions {
                        max_evaluation_work: work - 1,
                        ..opts
                    }
                ),
                Err(Error::ResourceLimit("bake evaluation work"))
            );
            counts.push(work);
        }
        assert!(counts[1] <= 10 * counts[0]);
    }
    #[test]
    fn bake_scaling_rotation_layers_and_animated_order() {
        let mut counts = Vec::new();
        for n in [32, 256] {
            let mut a = synthetic(1);
            a.layers[0].bindings.clear();
            a.node_order = vec![0, 1];
            a.values[0].default_value = [0.0, 20.0, 30.0];
            a.values.push(AnimationValue {
                element: 0,
                source_id: 0,
                default_value: [5.0, 0.0, 0.0],
                curves: [None; 3],
            });
            animated_property(&mut a, b"Lcl Rotation", 0);
            animated_property(&mut a, b"RotationOrder", 1);
            a.values.push(AnimationValue {
                element: 0,
                source_id: 0,
                default_value: [0.0; 3],
                curves: [None; 3],
            });
            a.layers[0].compose_rotation = true;
            for i in 1..n {
                let mut l = a.layers[0].clone();
                l.element = i + 2;
                for b in &mut l.bindings {
                    if b.property == b"RotationOrder" {
                        b.value = 2;
                    }
                }
                l.additive = true;
                l.weight = 0.01;
                a.layers.push(l);
                a.stacks[0].layers.push(i);
            }
            let layers = a.stack_layers(None).unwrap();
            let work = Cell::new(0);
            let rotation = a
                .bake_property(layers, 1, b"Lcl Rotation", 0.5, &|n| {
                    work.set(work.get() + n);
                    Ok(())
                })
                .unwrap();
            // One layer scan for rotation and one for RotationOrder, independent
            // of the number of later rotation layers.
            assert_eq!(work.get(), 2 * (n + 1));
            let pose = a.states(layers, 0.5).unwrap();
            let q = sn::euler_to_quat(
                [
                    rotation.components[0],
                    rotation.components[1],
                    rotation.components[2],
                ],
                5,
            );
            assert_eq!(pose[1].local.rotation, q);
            EVAL_WORK.with(|w| w.set(0));
            let b = bake(&a, None, &BakeOptions::default()).unwrap();
            assert!(!b.nodes[0].rotation_keys.is_empty());
            let work = EVAL_WORK.with(Cell::get);
            assert!(work < 1000 * n);
            eprintln!("bake rotation layers={n} work={work}");
            counts.push(work);
        }
        assert!(counts[1] <= 10 * counts[0]);
    }
    #[test]
    fn bake_reservation_sweep_and_work_cap() {
        let a = synthetic(3);
        let opts = BakeOptions::default();
        let mut passed = false;
        for n in 0..200 {
            crate::api_alloc::FAIL_AFTER.with(|f| f.set(Some(n)));
            let result = bake(&a, None, &opts);
            crate::api_alloc::FAIL_AFTER.with(|f| f.set(None));
            match result {
                Ok(_) => {
                    passed = true;
                    break;
                }
                Err(e) => assert_eq!(e, Error::Allocation, "{n}"),
            }
        }
        assert!(passed);
        let opts = BakeOptions {
            max_evaluation_work: 1,
            ..opts
        };
        assert!(matches!(
            bake(&a, None, &opts),
            Err(Error::ResourceLimit("bake evaluation work"))
        ));
    }
}
