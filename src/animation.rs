//! Animation records and deterministic evaluation, following ufbx 0.23.1.
use crate::fallible_map::Map as BTreeMap;
use crate::scene_node as sn;
use crate::{Error, Matrix, Transform};

/// Interpolation from a key to the next key.
///
/// Match with a fallback so future interpolation modes remain compatible:
/// ```compile_fail
/// # use fbx_rs::Interpolation;
/// let mode = Interpolation::Linear;
/// match mode {
///     Interpolation::ConstantPrevious | Interpolation::ConstantNext => {},
///     Interpolation::Linear | Interpolation::Cubic => {},
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum Interpolation {
    /// Hold the previous value.
    ConstantPrevious,
    /// Hold the next value between keys.
    ConstantNext,
    /// Linear interpolation.
    Linear,
    /// Cubic Bezier interpolation, including weighted time handles.
    Cubic,
}
/// A curve handle, stored at the source's single precision.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Tangent {
    /// Time displacement in seconds.
    pub dx: f32,
    /// Value displacement.
    pub dy: f32,
}
/// One authored curve key.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Keyframe {
    /// Source time in FBX ticks (46,186,158,000 per second).
    pub ticks: i64,
    /// Time in seconds.
    pub time: f64,
    /// Key value.
    pub value: f64,
    /// Interpolation to the next key.
    pub interpolation: Interpolation,
    /// Incoming handle.
    pub left: Tangent,
    /// Outgoing handle.
    pub right: Tangent,
}
/// Behaviour outside the curve's time range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum ExtrapolationMode {
    /// Hold the endpoint value.
    Constant,
    /// Continue the endpoint tangent.
    Slope = 3,
    /// Repeat the curve.
    Repeat = 1,
    /// Repeat and add the endpoint value difference.
    RepeatRelative = 4,
    /// Alternate forward and reversed repetitions.
    Mirror = 2,
}
/// Extrapolation settings for one side of a curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Extrapolation {
    /// Extrapolation operation.
    pub mode: ExtrapolationMode,
    /// Repetitions; -1 means unlimited.
    pub repeat_count: i32,
}
/// Owned animation curve; indices in animation values refer to this scene slice.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnimationCurve {
    pub(crate) element: usize,
    /// Original numeric object ID, or zero for name-based legacy records.
    pub source_id: i64,
    /// Authored keys in source order.
    pub keys: Vec<Keyframe>,
    /// Behaviour before the first key.
    pub pre_extrapolation: Extrapolation,
    /// Behaviour after the last key.
    pub post_extrapolation: Extrapolation,
}
impl Tangent {
    /// Construct a time/value handle.
    pub const fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }
}
impl Keyframe {
    /// Construct a linear key with zero tangent handles. Fields can be adjusted afterwards.
    pub fn new(ticks: i64, value: f64) -> Self {
        Self {
            ticks,
            time: ticks as f64 / KTIME,
            value,
            interpolation: Interpolation::Linear,
            left: Tangent::new(0.0, 0.0),
            right: Tangent::new(0.0, 0.0),
        }
    }
}
impl Extrapolation {
    /// Construct an extrapolation setting; negative counts mean unlimited repetitions.
    pub const fn new(mode: ExtrapolationMode, repeat_count: i32) -> Self {
        Self { mode, repeat_count }
    }
}
impl AnimationCurve {
    /// Evaluate in seconds. An empty curve returns `default_value`.
    /// Nonfinite times return an error.
    pub fn evaluate(&self, time: f64, default_value: f64) -> Result<f64, Error> {
        finite_time(time)?;
        Ok(self.eval(time, default_value, true))
    }
    pub(crate) fn eval(&self, time: f64, default_value: f64, extrapolate: bool) -> f64 {
        let keys = &self.keys;
        if keys.len() <= 1 {
            return keys.first().map_or(default_value, |k| k.value);
        }
        let first = &keys[0];
        let last = &keys[keys.len() - 1];
        if extrapolate && (time < first.time || time > last.time) {
            let pre = time < first.time;
            let (key, ext) = if pre {
                (first, self.pre_extrapolation)
            } else {
                (last, self.post_extrapolation)
            };
            match ext.mode {
                ExtrapolationMode::Constant => return key.value,
                ExtrapolationMode::Slope => {
                    let tangent = if pre { key.right } else { key.left };
                    return key.value
                        + f64::from(tangent.dy) * ((time - key.time) / f64::from(tangent.dx));
                }
                _ if ext.repeat_count == 0 => return key.value,
                _ => {}
            }
            let min = (first.time * KTIME).round_ties_even();
            let max = (last.time * KTIME).round_ties_even();
            let delta = if pre {
                min - time * KTIME
            } else {
                time * KTIME - max
            };
            let duration = max - min;
            if duration < 1.0 {
                return key.value;
            }
            let mut n = (delta / duration).floor();
            let mut d = delta - n * duration;
            if ext.repeat_count > 0 && n >= f64::from(ext.repeat_count) {
                n = f64::from(ext.repeat_count - 1);
                d = duration;
            }
            if ext.mode == ExtrapolationMode::Mirror && n * 0.5 - (n * 0.5).floor() <= 0.25 {
                d = duration - d;
            }
            if pre {
                d = duration - d;
            }
            let mut value = self.eval((min + d) / KTIME, key.value, false);
            if ext.mode == ExtrapolationMode::RepeatRelative {
                let delta = if pre {
                    -(last.value - first.value)
                } else {
                    last.value - first.value
                };
                value += delta * (n + 1.0);
            }
            return value;
        }
        let next = keys.partition_point(|k| k.time <= time);
        if next == 0 {
            return first.value;
        }
        if next == keys.len() {
            return last.value;
        }
        let p = &keys[next - 1];
        let n = &keys[next];
        if p.time == time {
            return p.value;
        }
        let rcp_delta = 1.0 / (n.time - p.time);
        let mut t = (time - p.time) * rcp_delta;
        match p.interpolation {
            Interpolation::ConstantPrevious => p.value,
            Interpolation::ConstantNext => n.value,
            Interpolation::Linear => p.value * (1.0 - t) + n.value * t,
            Interpolation::Cubic => {
                t = bezier_t(
                    f64::from(p.right.dx) * rcp_delta,
                    1.0 - f64::from(n.left.dx) * rcp_delta,
                    t,
                );
                let t2 = t * t;
                let t3 = t2 * t;
                let u = 1.0 - t;
                let u2 = u * u;
                let u3 = u2 * u;
                let y0 = p.value;
                let y3 = n.value;
                let y1 = y0 + f64::from(p.right.dy);
                let y2 = y3 - f64::from(n.left.dy);
                u3 * y0 + 3.0 * (u2 * t * y1 + u * t2 * y2) + t3 * y3
            }
        }
    }
}
pub(crate) const KTIME: f64 = 46_186_158_000.0;
fn bezier_t(handle1: f64, handle2: f64, x0: f64) -> f64 {
    let handle1_3 = handle1 * 3.0;
    let handle2_3 = handle2 * 3.0;
    let a = handle1_3 - handle2_3 + 1.0;
    let b = handle2_3 - handle1_3 - handle1_3;
    let c = handle1_3;
    let a_3 = 3.0 * a;
    let b_2 = 2.0 * b;
    let mut t = x0;
    let mut x1 = 0.0;
    for _ in 0..3 {
        let t2 = t * t;
        let t3 = t2 * t;
        x1 = a * t3 + b * t2 + c * t - x0;
        t -= x1 / (a_3 * t2 + b_2 * t + c);
    }
    if x1.abs() <= 8.881784197001252e-16 {
        return t;
    }
    for _ in 0..4 {
        for _ in 0..2 {
            let t2 = t * t;
            let t3 = t2 * t;
            x1 = a * t3 + b * t2 + c * t - x0;
            t -= x1 / (a_3 * t2 + b_2 * t + c);
        }
        if x1.abs() <= 8.881784197001252e-16 {
            return t;
        }
    }
    t
}
/// Three component curves and their fallback values.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnimationValue {
    pub(crate) element: usize,
    /// Original numeric object ID, or zero for name-based legacy records.
    pub source_id: i64,
    /// Values used for absent or empty component curves.
    pub default_value: [f64; 3],
    /// Component curve indices, in X/Y/Z order.
    pub curves: [Option<usize>; 3],
}
/// A property driven by an animation value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnimationBinding {
    pub(crate) element: usize,
    /// Numeric ID of the target object, or zero for a name-based legacy target.
    pub source_id: i64,
    /// Property name, preserving source bytes.
    pub property: Vec<u8>,
    /// Index into [`crate::Scene::animation_values`].
    pub value: usize,
}
/// One animation layer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnimationLayer {
    pub(crate) element: usize,
    /// Original numeric object ID, or zero for name-based legacy records.
    pub source_id: i64,
    /// Layer name.
    pub name: String,
    /// Loaded weight, from zero to one.
    pub weight: f64,
    /// The layer weight is animated.
    pub weight_is_animated: bool,
    /// Blend with the accumulated value.
    pub blended: bool,
    /// Add or compose onto the accumulated value.
    pub additive: bool,
    /// Compose rotations as quaternions.
    pub compose_rotation: bool,
    /// Compose scaling multiplicatively.
    pub compose_scale: bool,
    /// Animation values in connection order.
    pub values: Vec<usize>,
    /// Target properties, ordered by target and property.
    pub bindings: Vec<AnimationBinding>,
}
/// A named take and its ordered animation layers.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnimationStack {
    pub(crate) element: usize,
    /// Original numeric object ID, or zero for name-based legacy records.
    pub source_id: i64,
    /// Take name.
    pub name: String,
    /// Start time in seconds.
    pub time_begin: f64,
    /// End time in seconds.
    pub time_end: f64,
    /// Indices into [`crate::Scene::animation_layers`].
    pub layers: Vec<usize>,
}
/// Evaluated numeric and retained nonnumeric property values.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EvaluatedProperty {
    /// ufbx property type discriminant.
    pub property_type: u8,
    /// ufbx property flags.
    pub flags: u32,
    /// Integer view of the first component.
    pub value_int: i64,
    /// Numeric components (unused components are zero).
    pub components: [f64; 4],
    /// String value, retaining source bytes.
    pub value_str: Vec<u8>,
    /// Blob value.
    pub value_blob: Vec<u8>,
}
impl EvaluatedProperty {
    pub(crate) fn try_clone(&self, a: &crate::api_alloc::Budget) -> Result<Self, Error> {
        Ok(Self {
            property_type: self.property_type,
            flags: self.flags,
            value_int: self.value_int,
            components: self.components,
            value_str: a.bytes(&self.value_str)?,
            value_blob: a.bytes(&self.value_blob)?,
        })
    }
}

impl Default for EvaluatedProperty {
    fn default() -> Self {
        Self {
            property_type: 0,
            flags: 0x4000,
            value_int: 0,
            components: [0.0; 4],
            value_str: Vec::new(),
            value_blob: Vec::new(),
        }
    }
}
/// Transforms at one evaluation time. Indices match the loaded scene slices.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EvaluatedNode {
    /// Evaluated local transform.
    pub local_transform: Transform,
    /// Matrix from node space to parent space.
    pub node_to_parent: Matrix,
    /// Matrix from node space to world space.
    pub node_to_world: Matrix,
    /// Matrix from geometry space to node space.
    pub geometry_to_node: Matrix,
    /// Matrix from geometry space to world space.
    pub geometry_to_world: Matrix,
}
/// Owned scene pose at one time; source mesh arrays remain unchanged.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EvaluatedScene {
    /// Evaluated nodes, matching [`crate::Scene::nodes`].
    pub nodes: Vec<EvaluatedNode>,
    /// Evaluated channel weights, matching [`crate::Scene::blend_channels`].
    pub blend_weights: Vec<f64>,
    /// Evaluated cluster matrices, matching [`crate::Scene::skin_clusters`].
    pub skin_matrices: Vec<Matrix>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeEval {
    pub element: usize,
    pub parent: Option<usize>,
    pub props: sn::NodeProps,
    pub adjust: sn::Adjust,
    pub rotation_order: i64,
    pub use_rotation_space: bool,
    pub translation_scale: Option<usize>,
    pub inherited_scale: Option<usize>,
    /// `ufbx_node.is_scale_helper`.
    pub is_scale_helper: bool,
    /// Slot of `ufbx_node.scale_helper`.
    pub own_helper: Option<usize>,
    /// Slot of `ufbx_node.inherit_scale_node->scale_helper`.
    pub inherit_helper: Option<usize>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Animation {
    pub allocation_limit: usize,
    pub curves: Vec<AnimationCurve>,
    pub values: Vec<AnimationValue>,
    pub layers: Vec<AnimationLayer>,
    pub stacks: Vec<AnimationStack>,
    pub default_stack: Option<usize>,
    pub properties: BTreeMap<(usize, Vec<u8>), EvaluatedProperty>,
    pub nodes: Vec<NodeEval>,
    pub node_order: Vec<usize>,
    /// ufbx creation index (`element_id`) of each element, to order baked
    /// properties as ufbx does.
    pub creation: Vec<usize>,
    pub ids: Vec<Option<i64>>,
    pub channels: Vec<usize>,
    pub clusters: Vec<(Option<usize>, sn::Mat)>,
    pub connections: BTreeMap<(usize, Vec<u8>), (usize, Vec<u8>)>,
}
// Fixed transform input set: unrelated animated properties are never sampled.
const NODE_INPUTS: [&[u8]; 15] = [
    b"Lcl Translation",
    b"Lcl Rotation",
    b"Lcl Scaling",
    b"PreRotation",
    b"PostRotation",
    b"RotationOffset",
    b"RotationPivot",
    b"ScalingOffset",
    b"ScalingPivot",
    b"GeometricTranslation",
    b"GeometricRotation",
    b"GeometricScaling",
    b"RotationOrder",
    b"RotationActive",
    b"RotationSpaceForLimitOnly",
];
#[derive(Clone, Copy)]
pub(crate) struct NodeInputs {
    animated: u16,
    order: i64,
    active: i64,
    limit: i64,
}
#[derive(Clone, Copy)]
pub(crate) enum TransformChannel {
    Translation,
    Rotation,
    Scale,
}
impl TransformChannel {
    fn mask(self) -> u16 {
        match self {
            Self::Translation => 0x71ff, // local transform, order and rotation-space flags
            Self::Rotation => 0x701a,    // rotation, pre/post, order and rotation-space flags
            Self::Scale => 0x0004,
        }
    }
}
pub(crate) struct BakeSample<'a> {
    pub layers: &'a [usize],
    pub time: f64,
    pub work: &'a dyn Fn(usize) -> Result<(), Error>,
    /// `UFBX_TRANSFORM_FLAG_IGNORE_SCALE_HELPER`: the caller applies the
    /// scale helper's baked keys itself.
    pub ignore_scale_helper: bool,
}
struct PropertyContext<'a> {
    layers: &'a [usize],
    time: f64,
    retain_bytes: bool,
    rotation_order: Option<i64>,
    work: &'a dyn Fn(usize) -> Result<(), Error>,
}
// Borrow tuple keys without allocating a temporary property-name Vec.
trait PropertyKey {
    fn parts(&self) -> (usize, &[u8]);
}
impl PropertyKey for (usize, Vec<u8>) {
    fn parts(&self) -> (usize, &[u8]) {
        (self.0, &self.1)
    }
}
impl PropertyKey for (usize, &[u8]) {
    fn parts(&self) -> (usize, &[u8]) {
        *self
    }
}
impl PartialEq for dyn PropertyKey + '_ {
    fn eq(&self, b: &Self) -> bool {
        self.parts() == b.parts()
    }
}
impl Eq for dyn PropertyKey + '_ {}
impl PartialOrd for dyn PropertyKey + '_ {
    fn partial_cmp(&self, b: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for dyn PropertyKey + '_ {
    fn cmp(&self, b: &Self) -> std::cmp::Ordering {
        self.parts().cmp(&b.parts())
    }
}
impl<'a> std::borrow::Borrow<dyn PropertyKey + 'a> for (usize, Vec<u8>) {
    fn borrow(&self) -> &(dyn PropertyKey + 'a) {
        self
    }
}
fn lookup<'a, V>(
    map: &'a BTreeMap<(usize, Vec<u8>), V>,
    element: usize,
    name: &[u8],
) -> Option<&'a V> {
    map.get(&(element, name) as &dyn PropertyKey)
}
fn finite_time(time: f64) -> Result<(), Error> {
    if time.is_finite() {
        Ok(())
    } else {
        Err(Error::InvalidData("animation: nonfinite evaluation time"))
    }
}
impl Animation {
    pub fn stack_layers(&self, stack: Option<usize>) -> Result<&[usize], Error> {
        match stack {
            Some(i) => self
                .stacks
                .get(i)
                .map(|s| s.layers.as_slice())
                .ok_or(Error::InvalidData("animation: stack index out of range")),
            None => Ok(self
                .default_stack
                .map_or(&[], |i| self.stacks[i].layers.as_slice())),
        }
    }
    pub fn value(&self, i: usize, time: f64) -> [f64; 3] {
        let v = &self.values[i];
        std::array::from_fn(|c| {
            v.curves[c].map_or(v.default_value[c], |k| {
                self.curves[k].eval(time, v.default_value[c], true)
            })
        })
    }
    pub fn property(
        &self,
        layers: &[usize],
        element: usize,
        name: &[u8],
        time: f64,
    ) -> Result<EvaluatedProperty, Error> {
        self.property_impl(
            element,
            name,
            &PropertyContext {
                layers,
                time,
                retain_bytes: true,
                rotation_order: None,
                work: &|_| Ok(()),
            },
        )
    }
    pub(crate) fn numeric_property(
        &self,
        layers: &[usize],
        element: usize,
        name: &[u8],
        time: f64,
    ) -> Result<EvaluatedProperty, Error> {
        self.bake_property(layers, element, name, time, &|_| Ok(()))
    }
    pub(crate) fn bake_property(
        &self,
        layers: &[usize],
        element: usize,
        name: &[u8],
        time: f64,
        work: &dyn Fn(usize) -> Result<(), Error>,
    ) -> Result<EvaluatedProperty, Error> {
        self.property_impl(
            element,
            name,
            &PropertyContext {
                layers,
                time,
                retain_bytes: false,
                rotation_order: None,
                work,
            },
        )
    }
    fn property_impl(
        &self,
        element: usize,
        name: &[u8],
        ctx: &PropertyContext<'_>,
    ) -> Result<EvaluatedProperty, Error> {
        let PropertyContext {
            layers,
            time,
            retain_bytes,
            ..
        } = *ctx;
        (ctx.work)(1)?;
        let budget = crate::api_alloc::Budget::new(self.allocation_limit);
        let mut p = if let Some(p) = lookup(&self.properties, element, name) {
            EvaluatedProperty {
                property_type: p.property_type,
                flags: p.flags,
                value_int: p.value_int,
                components: p.components,
                value_str: if retain_bytes {
                    budget.bytes(&p.value_str)?
                } else {
                    Vec::new()
                },
                value_blob: if retain_bytes {
                    budget.bytes(&p.value_blob)?
                } else {
                    Vec::new()
                },
            }
        } else {
            EvaluatedProperty::default()
        };
        if p.flags & 0xa000 == 0 {
            return Ok(p);
        }
        // Follow property connections iteratively, as in ufbxi_evaluate_connected_prop.
        if p.flags & 0x8000 != 0 {
            let mut key = (element, name);
            let mut connected = false;
            for _ in 0..1000 {
                (ctx.work)(1)?;
                if let Some(next) = lookup(&self.connections, key.0, key.1) {
                    key = (next.0, &next.1);
                    connected = true;
                } else {
                    break;
                }
            }
            if connected && lookup(&self.connections, key.0, key.1).is_none() {
                let ep = self.property_impl(
                    key.0,
                    key.1,
                    &PropertyContext {
                        rotation_order: None,
                        ..*ctx
                    },
                )?;
                p.components = ep.components;
                p.value_int = ep.value_int;
                p.value_str = ep.value_str;
                p.value_blob = ep.value_blob;
                return Ok(p);
            }
            p.flags &= !0x8000;
        }
        let mut rotation_order = ctx.rotation_order;
        for (layer_ix, &ix) in layers.iter().enumerate() {
            (ctx.work)(1)?;
            let layer = &self.layers[ix];
            let binding_ix = layer
                .bindings
                .partition_point(|b| (b.element, b.property.as_slice()) < (element, name));
            let Some(binding) = layer
                .bindings
                .get(binding_ix)
                .filter(|b| b.element == element && b.property == name)
            else {
                continue;
            };
            let mut weight = layer.weight;
            if layer.weight_is_animated && layer.blended {
                let ix = layer
                    .bindings
                    .partition_point(|b| b.element < layer.element);
                if let Some(b) = layer
                    .bindings
                    .get(ix)
                    .filter(|b| b.element == layer.element)
                {
                    (ctx.work)(1)?;
                    weight = clamp_weight(self.value(b.value, time)[0] / 100.0);
                }
            }
            let value = self.value(binding.value, time);
            let mut result = [p.components[0], p.components[1], p.components[2]];
            if layer_ix == 0 {
                result = value;
            } else {
                let order = if name == b"Lcl Rotation" {
                    if let Some(order) = rotation_order {
                        order
                    } else {
                        let order = valid_order(
                            self.property_impl(element, b"RotationOrder", ctx)?
                                .value_int,
                        );
                        rotation_order = Some(order);
                        order
                    }
                } else {
                    0
                };
                combine(layer, weight, name, &mut result, value, order);
            }
            p.components[..3].copy_from_slice(&result);
        }
        p.value_int = crate::canon_dom::f64_to_i64(p.components[0]);
        Ok(p)
    }
    pub(crate) fn node_inputs(
        &self,
        i: usize,
        work: &dyn Fn(usize) -> Result<(), Error>,
    ) -> Result<NodeInputs, Error> {
        let node = &self.nodes[i];
        let mut input = NodeInputs {
            animated: 0,
            order: node.rotation_order,
            active: 1,
            limit: 0,
        };
        for (ix, name) in NODE_INPUTS.iter().enumerate() {
            work(1)?;
            if let Some(p) = lookup(&self.properties, node.element, name) {
                if p.flags & 0xa000 != 0 {
                    input.animated |= 1 << ix;
                }
                match ix {
                    12 => input.order = valid_order(p.value_int),
                    13 => input.active = p.value_int,
                    14 => input.limit = p.value_int,
                    _ => {}
                }
            }
        }
        Ok(input)
    }
    fn evaluated_props(
        &self,
        i: usize,
        input: &NodeInputs,
        mask: u16,
        ctx: &PropertyContext<'_>,
    ) -> Result<(sn::NodeProps, i64, bool), Error> {
        let node = &self.nodes[i];
        let mut props = node.props;
        let mut order = input.order;
        let mut active = input.active;
        let mut limit = input.limit;
        let animated = input.animated & mask;
        // Resolve RotationOrder once per sample, and reuse it for all rotation layers.
        if animated & (1 << 12) != 0 {
            order = valid_order(
                self.property_impl(node.element, b"RotationOrder", ctx)?
                    .value_int,
            );
        }
        let ctx = PropertyContext {
            rotation_order: Some(order),
            ..*ctx
        };
        for (ix, name) in NODE_INPUTS.iter().enumerate() {
            if ix == 12 || animated & (1 << ix) == 0 {
                continue;
            }
            let p = self.property_impl(node.element, name, &ctx)?;
            let v = [p.components[0], p.components[1], p.components[2]];
            match ix {
                0 => props.translation = v,
                1 => props.rotation = v,
                2 => props.scaling = v,
                3 => props.pre_rotation = v,
                4 => props.post_rotation = v,
                5 => props.rotation_offset = v,
                6 => props.rotation_pivot = v,
                7 => props.scaling_offset = v,
                8 => props.scaling_pivot = v,
                9 => props.geometric_translation = v,
                10 => props.geometric_rotation = v,
                11 => props.geometric_scaling = v,
                13 => active = p.value_int,
                14 => limit = p.value_int,
                _ => {}
            }
        }
        Ok((props, order, active != 0 && limit == 0))
    }
    fn evaluated_scale(
        &self,
        inputs: &[NodeInputs],
        mut i: usize,
        ctx: &PropertyContext<'_>,
    ) -> Result<[f64; 3], Error> {
        // Dependencies are already validated and acyclic at load time. Keep
        // evaluation iterative so deep inheritance cannot overflow the stack.
        let mut product = [1.0; 3];
        let mut count = 0;
        while i != 0 {
            count += 1;
            if count > self.nodes.len() {
                return Err(Error::ResourceLimit("animation scale cycle"));
            }
            (ctx.work)(1)?;
            let node = &self.nodes[i];
            let (props, order, use_rs) =
                self.evaluated_props(i, &inputs[i], TransformChannel::Scale.mask(), ctx)?;
            let local = sn::get_transform(&props, order, use_rs, &node.adjust, None);
            for (value, scale) in product.iter_mut().zip(local.scale) {
                *value *= scale;
            }
            let Some(next) = node.inherited_scale else {
                break;
            };
            i = next;
        }
        Ok(product)
    }
    pub(crate) fn bake_transform(
        &self,
        inputs: &[NodeInputs],
        i: usize,
        channel: TransformChannel,
        ctx: &BakeSample<'_>,
    ) -> Result<sn::Transform, Error> {
        if i == 0 {
            return Ok(sn::IDENTITY_TRANSFORM);
        }
        let ctx_ignore = ctx.ignore_scale_helper;
        let ctx = PropertyContext {
            layers: ctx.layers,
            time: ctx.time,
            retain_bytes: false,
            rotation_order: None,
            work: ctx.work,
        };
        (ctx.work)(1)?;
        let node = &self.nodes[i];
        let (props, order, use_rs) = self.evaluated_props(i, &inputs[i], channel.mask(), &ctx)?;
        let translation_scale = if matches!(channel, TransformChannel::Translation) && !ctx_ignore {
            node.translation_scale
                .map(|k| self.evaluated_scale(inputs, k, &ctx))
                .transpose()?
        } else {
            None
        };
        let mut local = sn::get_transform(&props, order, use_rs, &node.adjust, translation_scale);
        if matches!(channel, TransformChannel::Scale) && !ctx_ignore {
            if let Some(k) = node.inherited_scale {
                let scale = self.evaluated_scale(inputs, k, &ctx)?;
                for (value, scale) in local.scale.iter_mut().zip(scale) {
                    *value *= scale;
                }
            }
        }
        Ok(local)
    }
    pub fn states(&self, layers: &[usize], time: f64) -> Result<Vec<sn::NodeState>, Error> {
        let mut states = crate::api_alloc::Budget::new(self.allocation_limit)
            .filled(self.nodes.len(), sn::root_state())?;
        for &i in &self.node_order {
            let node = &self.nodes[i];
            if i == 0 {
                continue;
            }
            let input = self.node_inputs(i, &|_| Ok(()))?;
            let (props, order, use_rs) = self.evaluated_props(
                i,
                &input,
                u16::MAX,
                &PropertyContext {
                    layers,
                    time,
                    retain_bytes: false,
                    rotation_order: None,
                    work: &|_| Ok(()),
                },
            )?;
            let translation_scale = node.translation_scale.map(|i| states[i].local.scale);
            let mut local =
                sn::get_transform(&props, order, use_rs, &node.adjust, translation_scale);
            if let Some(i) = node.inherited_scale {
                for c in 0..3 {
                    local.scale[c] *= states[i].local.scale[c];
                }
            }
            let geometry = sn::get_geometry_transform(&props, &node.adjust);
            states[i] = sn::update_node(local, geometry, node.parent.map(|p| &states[p]));
        }
        Ok(states)
    }
    pub fn evaluate(&self, stack: Option<usize>, time: f64) -> Result<EvaluatedScene, Error> {
        finite_time(time)?;
        let layers = self.stack_layers(stack)?;
        let states = self.states(layers, time)?;
        let budget = crate::api_alloc::Budget::new(self.allocation_limit);
        Ok(EvaluatedScene {
            nodes: budget.collect(states.iter().map(|s| EvaluatedNode {
                local_transform: Transform {
                    translation: s.local.translation,
                    rotation: s.local.rotation,
                    scale: s.local.scale,
                },
                node_to_parent: Matrix(s.node_to_parent),
                node_to_world: Matrix(s.node_to_world),
                geometry_to_node: Matrix(s.geometry_to_node),
                geometry_to_world: Matrix(s.geometry_to_world),
            }))?,
            blend_weights: budget.try_collect(self.channels.iter().map(|&i| {
                Ok(self
                    .numeric_property(layers, i, b"DeformPercent", time)?
                    .components[0]
                    * 0.01)
            }))?,
            skin_matrices: budget.collect(self.clusters.iter().map(|(bone, bind)| {
                Matrix(bone.map_or(*bind, |i| sn::matrix_mul(&states[i].node_to_world, bind)))
            }))?,
        })
    }
}
pub(crate) fn clamp_weight(mut w: f64) -> f64 {
    if w < 0.0 {
        w = 0.0;
    }
    if w > f64::from(0.99999f32) {
        w = 1.0;
    }
    w
}
fn pow_abs(v: f64, e: f64) -> f64 {
    if e <= 0.0 {
        return 1.0;
    }
    if e >= 1.0 {
        return v;
    }
    let sign = if v < 0.0 { -1.0 } else { 1.0 };
    sign * (v * sign).powf(e)
}
fn combine(
    layer: &AnimationLayer,
    w: f64,
    name: &[u8],
    result: &mut [f64; 3],
    value: [f64; 3],
    order: i64,
) {
    let scale = layer.compose_scale && name == b"Lcl Scaling";
    let rotation = layer.compose_rotation && name == b"Lcl Rotation";
    if layer.additive {
        if scale {
            for i in 0..3 {
                result[i] *= pow_abs(value[i], w);
            }
        } else if rotation {
            *result = quat_to_euler(
                sn::mul_quat(
                    sn::euler_to_quat(*result, order),
                    slerp(sn::IDENTITY_QUAT, sn::euler_to_quat(value, order), w),
                ),
                order,
            );
        } else {
            for i in 0..3 {
                result[i] += value[i] * w;
            }
        }
    } else if layer.blended {
        let rw = 1.0 - w;
        if scale {
            for i in 0..3 {
                result[i] = pow_abs(result[i], rw) * pow_abs(value[i], w);
            }
        } else if rotation {
            *result = quat_to_euler(
                slerp(
                    sn::euler_to_quat(*result, order),
                    sn::euler_to_quat(value, order),
                    w,
                ),
                order,
            );
        } else {
            for i in 0..3 {
                result[i] = result[i] * rw + value[i] * w;
            }
        }
    } else {
        *result = value;
    }
}
pub(crate) fn slerp(a: sn::Quat, mut b: sn::Quat, t: f64) -> sn::Quat {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        dot = -dot;
        for v in &mut b {
            *v = -*v;
        }
    }
    let omega = dot.clamp(0.0, 1.0).acos();
    if omega <= f64::from(f32::MIN_POSITIVE) {
        return a;
    }
    let rcp = 1.0 / omega.sin();
    let af = ((1.0 - t) * omega).sin() * rcp;
    let bf = (t * omega).sin() * rcp;
    let [x, y, z, w] = std::array::from_fn(|i| af * a[i] + bf * b[i]);
    let rcp_len = 1.0 / (x * x + y * y + z * z + w * w).sqrt();
    [x * rcp_len, y * rcp_len, z * rcp_len, w * rcp_len]
}

// ufbx_quat_to_euler: all six Euler orders, including singular branches.
fn quat_to_euler(q: sn::Quat, order: i64) -> [f64; 3] {
    fn atan2(y: f64, x: f64) -> f64 {
        y.atan2(x)
    }
    let [qx, qy, qz, qw] = q;
    let eps = 0.999999999;
    let (vx, vy, vz): (f64, f64, f64);
    let t: f64;
    match order {
        0 => {
            t = 2.0 * (qw * qy - qx * qz);
            if t.abs() < eps {
                vy = t.asin();
                vz = atan2(2.0 * (qw * qz + qx * qy), 2.0 * (qw * qw + qx * qx) - 1.0);
                vx = -atan2(-2.0 * (qw * qx + qy * qz), 2.0 * (qw * qw + qz * qz) - 1.0);
            } else {
                vy = (std::f64::consts::PI * 0.5).copysign(t);
                vz = atan2(
                    -2.0 * t * (qw * qx - qy * qz),
                    t * (2.0 * qw * qy + 2.0 * qx * qz),
                );
                vx = 0.0;
            }
        }
        1 => {
            t = 2.0 * (qw * qz + qx * qy);
            if t.abs() < eps {
                vz = t.asin();
                vy = atan2(2.0 * (qw * qy - qx * qz), 2.0 * (qw * qw + qx * qx) - 1.0);
                vx = -atan2(-2.0 * (qw * qx - qy * qz), 2.0 * (qw * qw + qy * qy) - 1.0);
            } else {
                vz = (std::f64::consts::PI * 0.5).copysign(t);
                vy = atan2(
                    2.0 * t * (qw * qx + qy * qz),
                    -t * (2.0 * qx * qy - 2.0 * qw * qz),
                );
                vx = 0.0;
            }
        }
        2 => {
            t = 2.0 * (qw * qz - qx * qy);
            if t.abs() < eps {
                vz = t.asin();
                vx = atan2(2.0 * (qw * qx + qy * qz), 2.0 * (qw * qw + qy * qy) - 1.0);
                vy = -atan2(-2.0 * (qw * qy + qx * qz), 2.0 * (qw * qw + qx * qx) - 1.0);
            } else {
                vz = (std::f64::consts::PI * 0.5).copysign(t);
                vx = atan2(
                    -2.0 * t * (qw * qy - qx * qz),
                    t * (2.0 * qw * qz + 2.0 * qx * qy),
                );
                vy = 0.0;
            }
        }
        3 => {
            t = 2.0 * (qw * qx + qy * qz);
            if t.abs() < eps {
                vx = t.asin();
                vz = atan2(2.0 * (qw * qz - qx * qy), 2.0 * (qw * qw + qy * qy) - 1.0);
                vy = -atan2(-2.0 * (qw * qy - qx * qz), 2.0 * (qw * qw + qz * qz) - 1.0);
            } else {
                vx = (std::f64::consts::PI * 0.5).copysign(t);
                vz = atan2(
                    2.0 * t * (qw * qy + qx * qz),
                    -t * (2.0 * qy * qz - 2.0 * qw * qx),
                );
                vy = 0.0;
            }
        }
        4 => {
            t = 2.0 * (qw * qx - qy * qz);
            if t.abs() < eps {
                vx = t.asin();
                vy = atan2(2.0 * (qw * qy + qx * qz), 2.0 * (qw * qw + qz * qz) - 1.0);
                vz = -atan2(-2.0 * (qw * qz + qx * qy), 2.0 * (qw * qw + qy * qy) - 1.0);
            } else {
                vx = (std::f64::consts::PI * 0.5).copysign(t);
                vy = atan2(
                    -2.0 * t * (qw * qz - qx * qy),
                    t * (2.0 * qw * qx + 2.0 * qy * qz),
                );
                vz = 0.0;
            }
        }
        5 => {
            t = 2.0 * (qw * qy + qx * qz);
            if t.abs() < eps {
                vy = t.asin();
                vx = atan2(2.0 * (qw * qx - qy * qz), 2.0 * (qw * qw + qz * qz) - 1.0);
                vz = -atan2(-2.0 * (qw * qz - qx * qy), 2.0 * (qw * qw + qx * qx) - 1.0);
            } else {
                vy = (std::f64::consts::PI * 0.5).copysign(t);
                vx = atan2(
                    2.0 * t * (qw * qz + qx * qy),
                    -t * (2.0 * qx * qz - 2.0 * qw * qy),
                );
                vz = 0.0;
            }
        }
        _ => {
            vx = 0.0;
            vy = 0.0;
            vz = 0.0;
        }
    }

    [
        vx * (180.0 / std::f64::consts::PI),
        vy * (180.0 / std::f64::consts::PI),
        vz * (180.0 / std::f64::consts::PI),
    ]
}

fn valid_order(v: i64) -> i64 {
    if (0..=6).contains(&v) {
        v
    } else {
        0
    }
}
