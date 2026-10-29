//! Authored transform and global coordinate properties. Evaluation is not implemented.
//!
//! FBX property names and defaults follow the ufbx 0.23.1 property definitions:
//! <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
use crate::scene::{Error, SceneIndex};
use crate::syntax::{Document, Property};
use std::mem::size_of;

#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub translation: [f64; 3],
    pub rotation: [f64; 3],
    pub scaling: [f64; 3],
    pub rotation_offset: [f64; 3],
    pub rotation_pivot: [f64; 3],
    pub scaling_offset: [f64; 3],
    pub scaling_pivot: [f64; 3],
    pub pre_rotation: [f64; 3],
    pub post_rotation: [f64; 3],
    pub geometric_translation: [f64; 3],
    pub geometric_rotation: [f64; 3],
    pub geometric_scaling: [f64; 3],
    pub rotation_order: i64,
    pub inherit_type: i64,
    pub rotation_active: i64,
    pub rotation_space_for_limit_only: i64,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scaling: [1.0; 3],
            rotation_offset: [0.0; 3],
            rotation_pivot: [0.0; 3],
            scaling_offset: [0.0; 3],
            scaling_pivot: [0.0; 3],
            pre_rotation: [0.0; 3],
            post_rotation: [0.0; 3],
            geometric_translation: [0.0; 3],
            geometric_rotation: [0.0; 3],
            geometric_scaling: [1.0; 3],
            rotation_order: 0,
            inherit_type: -1,
            rotation_active: 1,
            rotation_space_for_limit_only: 0,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalSettings {
    pub up_axis: Option<i64>,
    pub up_axis_sign: Option<i64>,
    pub front_axis: Option<i64>,
    pub front_axis_sign: Option<i64>,
    pub coord_axis: Option<i64>,
    pub coord_axis_sign: Option<i64>,
    /// Centimetres per FBX unit: 1 means one centimetre per unit.
    pub unit_scale_factor: f64,
}
impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            up_axis: None,
            up_axis_sign: None,
            front_axis: None,
            front_axis_sign: None,
            coord_axis: None,
            coord_axis_sign: None,
            unit_scale_factor: 1.0,
        }
    }
}
#[derive(Debug)]
pub struct AuthoredTransforms {
    /// Indexed by object index; non-Model objects are None.
    pub models: Vec<Option<Transform>>,
    pub global_settings: GlobalSettings,
}
fn bytes<'a>(p: &'a Property<'_>) -> Option<&'a [u8]> {
    match p {
        Property::String(v) => Some(v),
        Property::AsciiBare(v) => Some(v),
        _ => None,
    }
}
fn number(p: &Property<'_>) -> Option<f64> {
    match p {
        Property::I16(v) => Some(f64::from(*v)),
        Property::I32(v) => Some(f64::from(*v)),
        Property::I64(v) => Some(*v as f64),
        Property::F32(v) => Some(f64::from(f32::from_bits(*v))),
        Property::F64(v) => Some(f64::from_bits(*v)),
        Property::AsciiNegativeZero => Some(-0.0),
        _ => None,
    }
}
fn integer(p: &Property<'_>) -> Option<i64> {
    match p {
        Property::Bool(v) | Property::BoolAlt(v) => Some(i64::from(*v != 0)),
        Property::I16(v) => Some(i64::from(*v)),
        Property::I32(v) => Some(i64::from(*v)),
        Property::I64(v) => Some(*v),
        _ => None,
    }
}
fn vector(p: &[Property<'_>]) -> Option<[f64; 3]> {
    Some([number(p.get(4)?)?, number(p.get(5)?)?, number(p.get(6)?)?])
}
fn apply_model(
    t: &mut Transform,
    name: &[u8],
    props: &[Property<'_>],
    i: usize,
) -> Result<(), Error> {
    let dest = match name {
        b"Lcl Translation" => Some(&mut t.translation),
        b"Lcl Rotation" => Some(&mut t.rotation),
        b"Lcl Scaling" => Some(&mut t.scaling),
        b"RotationOffset" => Some(&mut t.rotation_offset),
        b"RotationPivot" => Some(&mut t.rotation_pivot),
        b"ScalingOffset" => Some(&mut t.scaling_offset),
        b"ScalingPivot" => Some(&mut t.scaling_pivot),
        b"PreRotation" => Some(&mut t.pre_rotation),
        b"PostRotation" => Some(&mut t.post_rotation),
        b"GeometricTranslation" => Some(&mut t.geometric_translation),
        b"GeometricRotation" => Some(&mut t.geometric_rotation),
        b"GeometricScaling" => Some(&mut t.geometric_scaling),
        _ => None,
    };
    if let Some(dest) = dest {
        *dest = vector(props).ok_or(Error::InvalidObject { node: i })?;
    } else if name == b"RotationOrder"
        || name == b"InheritType"
        || name == b"RotationActive"
        || name == b"RotationSpaceForLimitOnly"
    {
        let v = props
            .get(4)
            .and_then(integer)
            .ok_or(Error::InvalidObject { node: i })?;
        match name {
            b"RotationOrder" => t.rotation_order = v,
            b"InheritType" => t.inherit_type = v,
            b"RotationActive" => t.rotation_active = v,
            _ => t.rotation_space_for_limit_only = v,
        }
    }
    Ok(())
}
/// Extract Properties70 values using the first Model definition as defaults.
/// The caller must not interpret this as a converted scene.
pub fn authored(
    doc: &Document<'_>,
    index: &SceneIndex<'_>,
    max_allocation: usize,
) -> Result<AuthoredTransforms, Error> {
    let count = index.objects.len();
    let requested = count
        .checked_mul(size_of::<Option<Transform>>())
        .ok_or(Error::LimitExceeded("allocation"))?;
    if requested > max_allocation {
        return Err(Error::LimitExceeded("allocation"));
    }
    let mut models = Vec::new();
    models
        .try_reserve_exact(count)
        .map_err(|_| Error::AllocationFailed)?;
    if models
        .capacity()
        .checked_mul(size_of::<Option<Transform>>())
        .ok_or(Error::LimitExceeded("allocation"))?
        > max_allocation
    {
        return Err(Error::LimitExceeded("allocation"));
    }
    // ufbx selects the first Model ObjectType and its first PropertyTemplate.
    let mut template = Transform::default();
    let mut template_node = None;
    for (i, n) in doc.nodes.iter().enumerate() {
        if n.name != b"ObjectType" {
            continue;
        }
        let Some(def) = n.parent.and_then(|p| doc.nodes.get(p)) else {
            continue;
        };
        if def.name != b"Definitions" || def.parent.is_some() {
            continue;
        }
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        if p.first().and_then(bytes) != Some(b"Model") {
            continue;
        }
        template_node = doc
            .nodes
            .iter()
            .enumerate()
            .find(|(_, c)| c.parent == Some(i) && c.name == b"PropertyTemplate")
            .map(|(j, _)| j);
        break;
    }
    if let Some(template_node) = template_node {
        for (i, n) in doc.nodes.iter().enumerate() {
            if n.name != b"P" {
                continue;
            }
            let Some(parent) = n.parent.and_then(|p| doc.nodes.get(p)) else {
                continue;
            };
            if parent.name != b"Properties70" || parent.parent != Some(template_node) {
                continue;
            }
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            if let Some(name) = p.first().and_then(bytes) {
                apply_model(&mut template, name, p, i)?;
            }
        }
    }
    for obj in &index.objects {
        let n = doc.nodes.get(obj.node).ok_or(Error::InvalidDom)?;
        models.push(if n.name == b"Model" {
            Some(template.clone())
        } else {
            None
        });
    }
    let mut global_settings = GlobalSettings::default();
    for (i, node) in doc.nodes.iter().enumerate() {
        if node.name != b"P" {
            continue;
        }
        let Some(parent) = node.parent.and_then(|n| doc.nodes.get(n)) else {
            continue;
        };
        if parent.name != b"Properties70" {
            continue;
        }
        let Some(owner_idx) = parent.parent else {
            continue;
        };
        let owner = doc.nodes.get(owner_idx).ok_or(Error::InvalidDom)?;
        let props = doc
            .properties
            .get(node.properties.clone())
            .ok_or(Error::InvalidDom)?;
        let Some(name) = props.first().and_then(bytes) else {
            continue;
        };
        if owner.name == b"Model"
            && owner
                .parent
                .and_then(|p| doc.nodes.get(p))
                .is_some_and(|p| p.name == b"Objects" && p.parent.is_none())
        {
            let Ok(obj) = index.objects.binary_search_by_key(&owner_idx, |o| o.node) else {
                return Err(Error::InvalidDom);
            };
            let t = models
                .get_mut(obj)
                .and_then(Option::as_mut)
                .ok_or(Error::InvalidDom)?;
            apply_model(t, name, props, i)?;
        } else if owner.name == b"GlobalSettings" && owner.parent.is_none() {
            let dest = match name {
                b"UpAxis" => Some(&mut global_settings.up_axis),
                b"UpAxisSign" => Some(&mut global_settings.up_axis_sign),
                b"FrontAxis" => Some(&mut global_settings.front_axis),
                b"FrontAxisSign" => Some(&mut global_settings.front_axis_sign),
                b"CoordAxis" => Some(&mut global_settings.coord_axis),
                b"CoordAxisSign" => Some(&mut global_settings.coord_axis_sign),
                _ => None,
            };
            if let Some(dest) = dest {
                *dest = Some(
                    props
                        .get(4)
                        .and_then(integer)
                        .ok_or(Error::InvalidObject { node: i })?,
                );
            } else if name == b"UnitScaleFactor" {
                global_settings.unit_scale_factor = props
                    .get(4)
                    .and_then(number)
                    .ok_or(Error::InvalidObject { node: i })?;
            }
        }
    }
    Ok(AuthoredTransforms {
        models,
        global_settings,
    })
}

/// Row-major affine matrix in source coordinates. It does not include parent
/// inheritance or the import profile's space-conversion adjustments.
pub type Matrix = [[f64; 4]; 4];

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for (k, row) in b.iter().enumerate() {
                out[i][j] += a[i][k] * row[j];
            }
        }
    }
    out
}
fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
fn translate(v: [f64; 3]) -> Matrix {
    let mut m = identity();
    for i in 0..3 {
        m[i][3] = v[i];
    }
    m
}
fn scale(v: [f64; 3]) -> Matrix {
    let mut m = identity();
    for i in 0..3 {
        m[i][i] = v[i];
    }
    m
}
fn negative(v: [f64; 3]) -> [f64; 3] {
    [-v[0], -v[1], -v[2]]
}
fn rotate(v: [f64; 3], order: i64) -> Result<Matrix, Error> {
    let axes = match order {
        0 => [0, 1, 2],
        1 => [0, 2, 1],
        2 => [1, 2, 0],
        3 => [1, 0, 2],
        4 => [2, 0, 1],
        5 => [2, 1, 0],
        6 => return Ok(identity()),
        _ => return Err(Error::InvalidTransform),
    };
    let mut result = identity();
    for axis in axes {
        let (s, c) = v[axis].to_radians().sin_cos();
        let mut m = identity();
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        m[a][a] = c;
        m[a][b] = -s;
        m[b][a] = s;
        m[b][b] = c;
        result = multiply(m, result);
    }
    Ok(result)
}
fn inverse_rotation(m: Matrix) -> Matrix {
    [
        [m[0][0], m[1][0], m[2][0], 0.0],
        [m[0][1], m[1][1], m[2][1], 0.0],
        [m[0][2], m[1][2], m[2][2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
impl Transform {
    /// The source-coordinate geometry transform affects attachments, not children.
    pub fn geometry_matrix(&self) -> Matrix {
        let rotation = rotate(self.geometric_rotation, 0).unwrap_or_else(|_| identity());
        multiply(
            multiply(translate(self.geometric_translation), rotation),
            scale(self.geometric_scaling),
        )
    }

    /// Evaluate the extracted local transform before axis/unit conversion.
    /// Derived from ufbx 0.23.1 `ufbxi_get_transform`: the post-rotation is inverse,
    /// and disabled rotation space uses XYZ without pre/post rotations.
    /// <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
    pub fn local_matrix(&self) -> Result<Matrix, Error> {
        let rotation = if self.rotation_active != 0 && self.rotation_space_for_limit_only == 0 {
            multiply(
                multiply(
                    rotate(self.pre_rotation, 0)?,
                    rotate(self.rotation, self.rotation_order)?,
                ),
                inverse_rotation(rotate(self.post_rotation, 0)?),
            )
        } else {
            rotate(self.rotation, 0)?
        };
        let parts = [
            translate(self.translation),
            translate(self.rotation_offset),
            translate(self.rotation_pivot),
            rotation,
            translate(negative(self.rotation_pivot)),
            translate(self.scaling_offset),
            translate(self.scaling_pivot),
            scale(self.scaling),
            translate(negative(self.scaling_pivot)),
        ];
        Ok(parts.into_iter().fold(identity(), multiply))
    }
}

// The scale state follows ufbx 0.23.1 `ufbxi_update_node` in preserve mode.
// https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c
#[derive(Clone, Copy)]
struct WorldState {
    unscaled: Matrix,
    inherit_scale: [f64; 3],
    scale_node: Option<usize>,
}
/// Compute source-coordinate world matrices with FBX's three inheritance modes.
/// The import profile's compensation and synthetic helper nodes are not
/// implemented here.
pub fn world_matrices(
    index: &SceneIndex<'_>,
    transforms: &AuthoredTransforms,
    max_allocation: usize,
) -> Result<Vec<Option<Matrix>>, Error> {
    let n = index.objects.len();
    if transforms.models.len() != n {
        return Err(Error::InvalidDom);
    }
    let per = size_of::<Option<Matrix>>() + size_of::<WorldState>() + size_of::<usize>();
    if n.checked_mul(per)
        .ok_or(Error::LimitExceeded("allocation"))?
        > max_allocation
    {
        return Err(Error::LimitExceeded("allocation"));
    }
    let mut world = Vec::new();
    let mut state = Vec::new();
    let mut path = Vec::new();
    world
        .try_reserve_exact(n)
        .map_err(|_| Error::AllocationFailed)?;
    state
        .try_reserve_exact(n)
        .map_err(|_| Error::AllocationFailed)?;
    path.try_reserve_exact(n)
        .map_err(|_| Error::AllocationFailed)?;
    let actual = world
        .capacity()
        .checked_mul(size_of::<Option<Matrix>>())
        .and_then(|v| {
            state
                .capacity()
                .checked_mul(size_of::<WorldState>())
                .and_then(|w| v.checked_add(w))
        })
        .and_then(|v| {
            path.capacity()
                .checked_mul(size_of::<usize>())
                .and_then(|w| v.checked_add(w))
        })
        .ok_or(Error::LimitExceeded("allocation"))?;
    if actual > max_allocation {
        return Err(Error::LimitExceeded("allocation"));
    }
    world.resize(n, None);
    state.resize(
        n,
        WorldState {
            unscaled: identity(),
            inherit_scale: [1.0; 3],
            scale_node: None,
        },
    );
    for start in 0..n {
        if transforms.models[start].is_none() || world[start].is_some() {
            continue;
        }
        path.clear();
        let mut next = Some(start);
        while let Some(i) = next {
            if i >= n || transforms.models[i].is_none() {
                return Err(Error::InvalidDom);
            }
            if world[i].is_some() {
                break;
            }
            if path.len() == n {
                return Err(Error::Cycle(index.objects[i].id));
            }
            path.push(i);
            next = index.objects[i].parent;
        }
        while let Some(i) = path.pop() {
            let t = transforms.models[i].as_ref().ok_or(Error::InvalidDom)?;
            let local = t.local_matrix()?;
            let mut unscaled_t = t.clone();
            unscaled_t.scaling = [1.0; 3];
            let mut unscaled_local = unscaled_t.local_matrix()?;
            for (row, value) in unscaled_local.iter_mut().zip(local.iter()).take(3) {
                row[3] = value[3];
            }
            let parent = index.objects[i].parent;
            let parent_world = match parent {
                Some(p) => world.get(p).and_then(|m| *m).ok_or(Error::InvalidDom)?,
                None => identity(),
            };
            let parent_state = match parent {
                Some(p) => *state.get(p).ok_or(Error::InvalidDom)?,
                None => WorldState {
                    unscaled: identity(),
                    inherit_scale: [1.0; 3],
                    scale_node: None,
                },
            };
            let special = t.inherit_type == 0 || t.inherit_type == 2;
            let scale_node = if t.inherit_type == 0 {
                parent
            } else if t.inherit_type == 2 {
                parent_state.scale_node
            } else {
                None
            };
            if special {
                let inherited = match scale_node {
                    Some(p) => state.get(p).ok_or(Error::InvalidDom)?.inherit_scale,
                    None => [1.0; 3],
                };
                let mut scaled = unscaled_local;
                let mut next_scale = [0.0; 3];
                for col in 0..3 {
                    next_scale[col] = t.scaling[col] * inherited[col];
                }
                for row in scaled.iter_mut().take(3) {
                    for col in 0..3 {
                        row[col] *= next_scale[col];
                    }
                }
                for row in 0..3 {
                    scaled[row][3] *= parent_state.inherit_scale[row];
                    unscaled_local[row][3] = scaled[row][3];
                }
                world[i] = Some(multiply(parent_state.unscaled, scaled));
                state[i] = WorldState {
                    unscaled: multiply(parent_state.unscaled, unscaled_local),
                    inherit_scale: next_scale,
                    scale_node,
                };
            } else {
                world[i] = Some(multiply(parent_world, local));
                state[i] = WorldState {
                    unscaled: multiply(parent_world, unscaled_local),
                    inherit_scale: t.scaling,
                    scale_node: None,
                };
            }
        }
    }
    Ok(world)
}

impl GlobalSettings {
    /// Source coordinates to +X right, +Y up, +Z front, including centimetres
    /// to metres. Returns None for missing/duplicate axes or invalid units.
    /// This basis alone does not implement ufbx's handedness adjustment policy.
    pub fn source_to_y_up_meters(&self) -> Option<Matrix> {
        let axes = [self.coord_axis?, self.up_axis?, self.front_axis?];
        let signs = [
            self.coord_axis_sign.unwrap_or(2),
            self.up_axis_sign.unwrap_or(2),
            self.front_axis_sign.unwrap_or(2),
        ];
        if !self.unit_scale_factor.is_finite() || self.unit_scale_factor <= 0.0 {
            return None;
        }
        let unit = self.unit_scale_factor * 0.01;
        if !unit.is_finite() || unit == 0.0 {
            return None;
        }
        let mut used = [false; 3];
        let mut result = [[0.0; 4]; 4];
        result[3][3] = 1.0;
        for i in 0..3 {
            let axis = usize::try_from(axes[i]).ok()?;
            if axis >= 3 || used[axis] {
                return None;
            }
            used[axis] = true;
            result[i][axis] = unit * if signs[i] > 0 { 1.0 } else { -1.0 };
        }
        Some(result)
    }
}

/// Coordinate-basis conversion helper. This does not implement ufbx's
/// helper-node, instancing, normal normalization, or winding policies.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceConversion {
    pub source_to_target: Matrix,
    pub target_to_source: Matrix,
    pub reverses_handedness: bool,
}
impl GlobalSettings {
    pub fn y_up_meters_conversion(&self) -> Option<SpaceConversion> {
        let forward = self.source_to_y_up_meters()?;
        let mut inverse = [[0.0; 4]; 4];
        inverse[3][3] = 1.0;
        for (i, row) in forward.iter().enumerate().take(3) {
            for (j, &element) in row.iter().enumerate().take(3) {
                if element != 0.0 {
                    let v = 1.0 / element;
                    if !v.is_finite() {
                        return None;
                    }
                    inverse[j][i] = v;
                }
            }
        }
        let normalized = self.unit_scale_factor * 0.01;
        let f = forward.map(|r| r.map(|v| v / normalized));
        let determinant = f[0][0] * (f[1][1] * f[2][2] - f[1][2] * f[2][1])
            - f[0][1] * (f[1][0] * f[2][2] - f[1][2] * f[2][0])
            + f[0][2] * (f[1][0] * f[2][1] - f[1][1] * f[2][0]);
        Some(SpaceConversion {
            source_to_target: forward,
            target_to_source: inverse,
            reverses_handedness: determinant < 0.0,
        })
    }
}
impl SpaceConversion {
    pub fn matrix(&self, source: Matrix) -> Matrix {
        multiply(
            multiply(self.source_to_target, source),
            self.target_to_source,
        )
    }
    pub fn point(&self, source: [f64; 3]) -> [f64; 3] {
        let mut result = [0.0; 3];
        for (i, value) in result.iter_mut().enumerate() {
            *value = self.source_to_target[i][0] * source[0]
                + self.source_to_target[i][1] * source[1]
                + self.source_to_target[i][2] * source[2];
        }
        result
    }
}

impl GlobalSettings {
    /// Preliminary import-profile world projection for a node with no generated
    /// helpers or camera/light axis adjustments. Returns None when the source
    /// axes or unit are unknown. Full scene conversion must handle those cases.
    pub fn project_plain_world_y_up(&self, world: Matrix) -> Option<Matrix> {
        let conversion = self.y_up_meters_conversion()?;
        let unit = self.unit_scale_factor * 0.01;
        let mut axes = conversion.source_to_target;
        for row in axes.iter_mut().take(3) {
            for value in row.iter_mut().take(3) {
                *value /= unit;
            }
        }
        let mut output = multiply(axes, world);
        for row in output.iter_mut().take(3) {
            row[3] *= unit;
        }
        if conversion.reverses_handedness {
            for row in output.iter_mut().take(3) {
                row[2] = -row[2];
            }
        }
        Some(output)
    }

    /// Apply the pinned import profile camera/light axis adjustment to a world matrix
    /// when the caller has already established a single corresponding node
    /// attribute, no descendants affected by the adjustment, and the correct
    /// effective local scale (including any inherited scale). Zero or nonfinite
    /// scale components are rejected because the matrix cannot be factored.
    /// This does not implement generated helpers or inherited adjustment
    /// propagation. In particular, it is not a general camera/light converter.
    /// Axis conventions follow ufbx 0.23.1 `ufbxi_update_adjust_transforms`:
    /// <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
    pub fn project_leaf_attribute_world_y_up(
        &self,
        world: Matrix,
        attribute: LeafAttribute,
        effective_local_scale: [f64; 3],
    ) -> Option<Matrix> {
        if !effective_local_scale
            .iter()
            .all(|v| v.is_finite() && *v != 0.0)
        {
            return None;
        }
        let post = match attribute {
            LeafAttribute::Camera => [
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            LeafAttribute::Light => [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let mut unscaled = world;
        for row in unscaled.iter_mut().take(3) {
            for col in 0..3 {
                row[col] /= effective_local_scale[col];
            }
        }
        let mut adjusted = multiply(unscaled, post);
        for row in adjusted.iter_mut().take(3) {
            for col in 0..3 {
                row[col] *= effective_local_scale[col];
            }
        }
        self.project_plain_world_y_up(adjusted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafAttribute {
    Camera,
    Light,
}
