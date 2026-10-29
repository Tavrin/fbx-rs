//! Node transforms of the engine-converted profile, ported operation for
//! operation from pinned ufbx 0.23.1 (`ufbx.c`, `ufbx_real` = `double`).
//!
//! Load order reproduced: `ufbxi_pre_finalize_scene` inherit-mode analysis,
//! `ufbxi_transform_to_axes`, `ufbxi_scale_units`,
//! `ufbxi_update_adjust_transforms`, `ufbxi_modify_geometry` (geometric
//! property reset) and `ufbxi_update_node`. Every expression keeps the C
//! association order so results are bit-identical (no fused multiply-add; the
//! reference build uses `-ffp-contract=off` and libm `sin`/`cos`/`sqrt`).

pub type Vec3 = [f64; 3];
/// Quaternion `x, y, z, w`.
pub type Quat = [f64; 4];
/// `ufbx_matrix.v`: column-major `m00 m10 m20 m01 m11 m21 m02 m12 m22 m03 m13 m23`.
pub type Mat = [f64; 12];

/// `ufbx_transform`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

pub const IDENTITY_QUAT: Quat = [0.0, 0.0, 0.0, 1.0];
pub const IDENTITY_TRANSFORM: Transform = Transform {
    translation: [0.0; 3],
    rotation: IDENTITY_QUAT,
    scale: [1.0; 3],
};
pub const IDENTITY_MATRIX: Mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
/// `UFBX_EPSILON` for double `ufbx_real`.
const EPSILON: f64 = 1.491_668_146_240_041_3e-154;

fn is_vec3_zero(v: Vec3) -> bool {
    v[0] == 0.0 && v[1] == 0.0 && v[2] == 0.0
}
fn is_vec3_one(v: Vec3) -> bool {
    v[0] == 1.0 && v[1] == 1.0 && v[2] == 1.0
}
fn is_quat_identity(q: Quat) -> bool {
    q[0] == 0.0 && q[1] == 0.0 && q[2] == 0.0 && q[3] == 1.0
}
/// `ufbxi_is_transform_identity`.
pub fn is_transform_identity(t: &Transform) -> bool {
    is_vec3_zero(t.translation) && is_quat_identity(t.rotation) && is_vec3_one(t.scale)
}

/// `ufbxi_mul_quat`.
pub fn mul_quat(a: Quat, b: Quat) -> Quat {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `ufbx_quat_rotate_vec3`.
pub fn quat_rotate_vec3(q: Quat, v: Vec3) -> Vec3 {
    let (qx, qy, qz, qw) = (q[0], q[1], q[2], q[3]);
    let xy = qx * v[1] - qy * v[0];
    let xz = qx * v[2] - qz * v[0];
    let yz = qy * v[2] - qz * v[1];
    [
        2.0 * (qw * yz + qy * xy + qz * xz) + v[0],
        2.0 * (-qx * xy - qw * xz + qz * yz) + v[1],
        2.0 * (-qx * xz - qy * yz + qw * xy) + v[2],
    ]
}

/// `ufbx_euler_to_quat`; orders outside XYZ..ZYX (SPHERIC) give identity.
pub fn euler_to_quat(v: Vec3, order: i64) -> Quat {
    // `UFBXI_DEG_TO_RAD_DOUBLE * 0.5`; the C literal rounds to `PI`.
    const HALF_DEG: f64 = (std::f64::consts::PI / 180.0) * 0.5;
    let vx = v[0] * HALF_DEG;
    let vy = v[1] * HALF_DEG;
    let vz = v[2] * HALF_DEG;
    let (cx, sx) = (vx.cos(), vx.sin());
    let (cy, sy) = (vy.cos(), vy.sin());
    let (cz, sz) = (vz.cos(), vz.sin());
    match order {
        0 => [
            -cx * sy * sz + cy * cz * sx,
            cx * cz * sy + cy * sx * sz,
            cx * cy * sz - cz * sx * sy,
            cx * cy * cz + sx * sy * sz,
        ],
        1 => [
            cx * sy * sz + cy * cz * sx,
            cx * cz * sy + cy * sx * sz,
            cx * cy * sz - cz * sx * sy,
            cx * cy * cz - sx * sy * sz,
        ],
        2 => [
            -cx * sy * sz + cy * cz * sx,
            cx * cz * sy - cy * sx * sz,
            cx * cy * sz + cz * sx * sy,
            cx * cy * cz + sx * sy * sz,
        ],
        3 => [
            -cx * sy * sz + cy * cz * sx,
            cx * cz * sy + cy * sx * sz,
            cx * cy * sz + cz * sx * sy,
            cx * cy * cz - sx * sy * sz,
        ],
        4 => [
            cx * sy * sz + cy * cz * sx,
            cx * cz * sy - cy * sx * sz,
            cx * cy * sz - cz * sx * sy,
            cx * cy * cz + sx * sy * sz,
        ],
        5 => [
            cx * sy * sz + cy * cz * sx,
            cx * cz * sy - cy * sx * sz,
            cx * cy * sz + cz * sx * sy,
            cx * cy * cz - sx * sy * sz,
        ],
        _ => IDENTITY_QUAT,
    }
}

/// `ufbx_transform_to_matrix`.
pub fn transform_to_matrix(t: &Transform) -> Mat {
    let q = t.rotation;
    let (sx, sy, sz) = (2.0 * t.scale[0], 2.0 * t.scale[1], 2.0 * t.scale[2]);
    let (xx, xy, xz, xw) = (q[0] * q[0], q[0] * q[1], q[0] * q[2], q[0] * q[3]);
    let (yy, yz, yw) = (q[1] * q[1], q[1] * q[2], q[1] * q[3]);
    let (zz, zw) = (q[2] * q[2], q[2] * q[3]);
    [
        sx * (-yy - zz + 0.5),
        sx * (xy + zw),
        sx * (-yw + xz),
        sy * (-zw + xy),
        sy * (-xx - zz + 0.5),
        sy * (xw + yz),
        sz * (xz + yw),
        sz * (-xw + yz),
        sz * (-xx - yy + 0.5),
        t.translation[0],
        t.translation[1],
        t.translation[2],
    ]
}

/// `ufbxi_unscaled_transform_to_matrix`.
pub fn unscaled_transform_to_matrix(t: &Transform) -> Mat {
    let mut u = *t;
    u.scale = [1.0; 3];
    transform_to_matrix(&u)
}

// Matrix element accessors (row, column) over the column-major layout.
const fn ix(r: usize, c: usize) -> usize {
    c * 3 + r
}

/// `ufbx_matrix_mul`.
pub fn matrix_mul(a: &Mat, b: &Mat) -> Mat {
    let g = |m: &Mat, r: usize, c: usize| m[ix(r, c)];
    let mut d = [0.0; 12];
    for r in 0..3 {
        d[ix(r, 3)] = g(a, r, 0) * g(b, 0, 3)
            + g(a, r, 1) * g(b, 1, 3)
            + g(a, r, 2) * g(b, 2, 3)
            + g(a, r, 3);
    }
    for c in 0..3 {
        for r in 0..3 {
            d[ix(r, c)] =
                g(a, r, 0) * g(b, 0, c) + g(a, r, 1) * g(b, 1, c) + g(a, r, 2) * g(b, 2, c);
        }
    }
    d
}

/// `ufbx_matrix_determinant`.
pub fn matrix_determinant(m: &Mat) -> f64 {
    let g = |r: usize, c: usize| m[ix(r, c)];
    -g(0, 2) * g(1, 1) * g(2, 0) + g(0, 1) * g(1, 2) * g(2, 0) + g(0, 2) * g(1, 0) * g(2, 1)
        - g(0, 0) * g(1, 2) * g(2, 1)
        - g(0, 1) * g(1, 0) * g(2, 2)
        + g(0, 0) * g(1, 1) * g(2, 2)
}

/// `ufbx_matrix_invert`.
pub fn matrix_invert(m: &Mat) -> Mat {
    let det = matrix_determinant(m);
    if det.abs() <= EPSILON {
        return [0.0; 12];
    }
    let g = |r: usize, c: usize| m[ix(r, c)];
    let rcp = 1.0 / det;
    let mut r = [0.0; 12];
    r[ix(0, 0)] = (-g(1, 2) * g(2, 1) + g(1, 1) * g(2, 2)) * rcp;
    r[ix(1, 0)] = (g(1, 2) * g(2, 0) - g(1, 0) * g(2, 2)) * rcp;
    r[ix(2, 0)] = (-g(1, 1) * g(2, 0) + g(1, 0) * g(2, 1)) * rcp;
    r[ix(0, 1)] = (g(0, 2) * g(2, 1) - g(0, 1) * g(2, 2)) * rcp;
    r[ix(1, 1)] = (-g(0, 2) * g(2, 0) + g(0, 0) * g(2, 2)) * rcp;
    r[ix(2, 1)] = (g(0, 1) * g(2, 0) - g(0, 0) * g(2, 1)) * rcp;
    r[ix(0, 2)] = (-g(0, 2) * g(1, 1) + g(0, 1) * g(1, 2)) * rcp;
    r[ix(1, 2)] = (g(0, 2) * g(1, 0) - g(0, 0) * g(1, 2)) * rcp;
    r[ix(2, 2)] = (-g(0, 1) * g(1, 0) + g(0, 0) * g(1, 1)) * rcp;
    r[ix(0, 3)] =
        (g(0, 3) * g(1, 2) * g(2, 1) - g(0, 2) * g(1, 3) * g(2, 1) - g(0, 3) * g(1, 1) * g(2, 2)
            + g(0, 1) * g(1, 3) * g(2, 2)
            + g(0, 2) * g(1, 1) * g(2, 3)
            - g(0, 1) * g(1, 2) * g(2, 3))
            * rcp;
    r[ix(1, 3)] = (g(0, 2) * g(1, 3) * g(2, 0) - g(0, 3) * g(1, 2) * g(2, 0)
        + g(0, 3) * g(1, 0) * g(2, 2)
        - g(0, 0) * g(1, 3) * g(2, 2)
        - g(0, 2) * g(1, 0) * g(2, 3)
        + g(0, 0) * g(1, 2) * g(2, 3))
        * rcp;
    r[ix(2, 3)] =
        (g(0, 3) * g(1, 1) * g(2, 0) - g(0, 1) * g(1, 3) * g(2, 0) - g(0, 3) * g(1, 0) * g(2, 1)
            + g(0, 0) * g(1, 3) * g(2, 1)
            + g(0, 1) * g(1, 0) * g(2, 3)
            - g(0, 0) * g(1, 1) * g(2, 3))
            * rcp;
    r
}

/// `ufbx_transform_direction`.
pub fn transform_direction(m: &Mat, v: Vec3) -> Vec3 {
    let g = |r: usize, c: usize| m[ix(r, c)];
    [
        g(0, 0) * v[0] + g(0, 1) * v[1] + g(0, 2) * v[2],
        g(1, 0) * v[0] + g(1, 1) * v[1] + g(1, 2) * v[2],
        g(2, 0) * v[0] + g(2, 1) * v[1] + g(2, 2) * v[2],
    ]
}

fn length3(v: Vec3) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `ufbx_matrix_to_transform`.
pub fn matrix_to_transform(m: &Mat) -> Transform {
    let det = matrix_determinant(m);
    let col = |c: usize| [m[c * 3], m[c * 3 + 1], m[c * 3 + 2]];
    let mut t = IDENTITY_TRANSFORM;
    t.translation = col(3);
    t.scale = [length3(col(0)), length3(col(1)), length3(col(2))];
    let (mut sign_x, mut sign_y, mut sign_z) = (1.0, 1.0, 1.0);
    if det < 0.0 {
        if t.scale[0] > 0.0 {
            sign_x = -1.0;
        } else if t.scale[1] > 0.0 {
            sign_y = -1.0;
        } else if t.scale[2] > 0.0 {
            sign_z = -1.0;
        }
    }
    let mul3 = |v: Vec3, s: f64| [v[0] * s, v[1] * s, v[2] * s];
    let f = |s: f64, sign: f64| if s > 0.0 { sign / s } else { 0.0 };
    let x = mul3(col(0), f(t.scale[0], sign_x));
    let y = mul3(col(1), f(t.scale[1], sign_y));
    let z = mul3(col(2), f(t.scale[2], sign_z));
    let trace = x[0] + y[1] + z[2];
    let half = |a: f64| if a != 0.0 { 0.5 / a } else { 0.0 };
    let r = &mut t.rotation;
    if trace > 0.0 {
        let a = (trace + 1.0).max(0.0).sqrt();
        let b = half(a);
        *r = [
            (y[2] - z[1]) * b,
            (z[0] - x[2]) * b,
            (x[1] - y[0]) * b,
            0.5 * a,
        ];
    } else if x[0] > y[1] && x[0] > z[2] {
        let a = (1.0 + x[0] - y[1] - z[2]).max(0.0).sqrt();
        let b = half(a);
        *r = [
            0.5 * a,
            (y[0] + x[1]) * b,
            (z[0] + x[2]) * b,
            (y[2] - z[1]) * b,
        ];
    } else if y[1] > z[2] {
        let a = (1.0 - x[0] + y[1] - z[2]).max(0.0).sqrt();
        let b = half(a);
        *r = [
            (y[0] + x[1]) * b,
            0.5 * a,
            (z[1] + y[2]) * b,
            (z[0] - x[2]) * b,
        ];
    } else {
        let a = (1.0 - x[0] - y[1] + z[2]).max(0.0).sqrt();
        let b = half(a);
        *r = [
            (z[0] + x[2]) * b,
            (z[1] + y[2]) * b,
            0.5 * a,
            (x[1] - y[0]) * b,
        ];
    }
    let len = r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3];
    if (len - 1.0).abs() > EPSILON {
        if len.abs() <= EPSILON {
            *r = IDENTITY_QUAT;
        } else {
            for c in r.iter_mut() {
                *c /= len;
            }
        }
    }
    t.scale[0] *= sign_x;
    t.scale[1] *= sign_y;
    t.scale[2] *= sign_z;
    t
}

/// `ufbxi_axis_matrix`: `None` when no conversion is needed.
pub fn axis_matrix(src: [i64; 3], dst: [i64; 3]) -> Option<Mat> {
    if src == dst {
        return None;
    }
    let mut m = [0.0; 12];
    for (s, d) in src.into_iter().zip(dst) {
        let (s, d) = (usize::try_from(s).ok()?, usize::try_from(d).ok()?);
        m[(s >> 1) * 3 + (d >> 1)] = if (s ^ d) & 1 == 0 { 1.0 } else { -1.0 };
    }
    Some(m)
}

/// `ufbx_coordinate_axes_valid` over `[right, up, front]`.
pub fn axes_valid(axes: [i64; 3]) -> bool {
    if axes.iter().any(|a| !(0..=5).contains(a)) {
        return false;
    }
    axes.iter().fold(0u32, |m, &a| m | 1 << (a >> 1)) & 7 == 7
}

/// `ufbxi_mirror_matrix_dst`.
pub fn mirror_matrix_dst(m: &mut Mat, axis: i64) {
    if axis == 0 {
        return;
    }
    let a = usize::try_from(axis - 1).expect("validated mirror axis");
    for c in 0..4 {
        m[c * 3 + a] = -m[c * 3 + a];
    }
}

fn mirror_translation(v: &mut Vec3, axis: i64) {
    let a = usize::try_from(axis - 1).expect("validated mirror axis");
    v[a] = -v[a];
}
fn mirror_rotation(q: &mut Quat, axis: i64) {
    let a = usize::try_from(axis).expect("validated axis");
    q[a % 3] = -q[a % 3];
    q[(a + 1) % 3] = -q[(a + 1) % 3];
}

pub(crate) fn add_translate(t: &mut Transform, v: Vec3) {
    for (i, x) in v.into_iter().enumerate() {
        t.translation[i] += x;
    }
}
pub(crate) fn sub_translate(t: &mut Transform, v: Vec3) {
    for (i, x) in v.into_iter().enumerate() {
        t.translation[i] -= x;
    }
}
pub(crate) fn mul_scale(t: &mut Transform, v: Vec3) {
    for (i, x) in v.into_iter().enumerate() {
        t.translation[i] *= x;
    }
    for (i, x) in v.into_iter().enumerate() {
        t.scale[i] *= x;
    }
}
fn mul_scale_real(t: &mut Transform, v: f64) {
    mul_scale(t, [v; 3]);
}
fn rotate_by(t: &mut Transform, q: Quat) {
    if t.rotation[3] != 1.0 {
        t.rotation = mul_quat(q, t.rotation);
    } else {
        t.rotation = q;
    }
    if !is_vec3_zero(t.translation) {
        t.translation = quat_rotate_vec3(q, t.translation);
    }
}
pub(crate) fn mul_rotate(t: &mut Transform, v: Vec3, order: i64) {
    if is_vec3_zero(v) {
        return;
    }
    rotate_by(t, euler_to_quat(v, order));
}
fn mul_rotate_quat(t: &mut Transform, q: Quat) {
    if is_quat_identity(q) {
        return;
    }
    rotate_by(t, q);
}
fn mul_inv_rotate(t: &mut Transform, v: Vec3, order: i64) {
    if is_vec3_zero(v) {
        return;
    }
    let q = euler_to_quat(v, order);
    rotate_by(t, [-q[0], -q[1], -q[2], q[3]]);
}

/// Adjustment state of one node (`ufbx_node` adjust fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    pub has_adjust_transform: bool,
    pub has_root_adjust_transform: bool,
    pub pre_translation: Vec3,
    pub pre_rotation: Quat,
    pub pre_scale: f64,
    pub post_rotation: Quat,
    pub post_scale: f64,
    pub translation_scale: f64,
    pub mirror_axis: i64,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            has_adjust_transform: false,
            has_root_adjust_transform: false,
            pre_translation: [0.0; 3],
            pre_rotation: IDENTITY_QUAT,
            pre_scale: 1.0,
            post_rotation: IDENTITY_QUAT,
            post_scale: 1.0,
            translation_scale: 1.0,
            mirror_axis: 0,
        }
    }
}

/// Transform-relevant node properties, already resolved through the
/// template defaults chain (`ufbxi_find_vec3` with its literal defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeProps {
    pub translation: Vec3,
    pub rotation: Vec3,
    pub scaling: Vec3,
    pub pre_rotation: Vec3,
    pub post_rotation: Vec3,
    pub rotation_offset: Vec3,
    pub rotation_pivot: Vec3,
    pub scaling_offset: Vec3,
    pub scaling_pivot: Vec3,
    pub geometric_translation: Vec3,
    pub geometric_rotation: Vec3,
    pub geometric_scaling: Vec3,
}

impl Default for NodeProps {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scaling: [1.0; 3],
            pre_rotation: [0.0; 3],
            post_rotation: [0.0; 3],
            rotation_offset: [0.0; 3],
            rotation_pivot: [0.0; 3],
            scaling_offset: [0.0; 3],
            scaling_pivot: [0.0; 3],
            geometric_translation: [0.0; 3],
            geometric_rotation: [0.0; 3],
            geometric_scaling: [1.0; 3],
        }
    }
}

/// `ufbxi_get_transform`.
pub fn get_transform(
    p: &NodeProps,
    order: i64,
    use_rotation_space: bool,
    adj: &Adjust,
    translation_scale: Option<Vec3>,
) -> Transform {
    let mut translation = p.translation;
    if let Some(s) = translation_scale {
        for i in 0..3 {
            translation[i] *= s[i];
        }
    }
    let mut t = IDENTITY_TRANSFORM;
    if adj.has_adjust_transform {
        mul_rotate_quat(&mut t, adj.post_rotation);
        mul_scale_real(&mut t, adj.post_scale);
    }
    sub_translate(&mut t, p.scaling_pivot);
    mul_scale(&mut t, p.scaling);
    add_translate(&mut t, p.scaling_pivot);
    add_translate(&mut t, p.scaling_offset);
    sub_translate(&mut t, p.rotation_pivot);
    if use_rotation_space {
        mul_inv_rotate(&mut t, p.post_rotation, 0);
        mul_rotate(&mut t, p.rotation, order);
        mul_rotate(&mut t, p.pre_rotation, 0);
    } else {
        mul_rotate(&mut t, p.rotation, 0);
    }
    add_translate(&mut t, p.rotation_pivot);
    add_translate(&mut t, p.rotation_offset);
    add_translate(&mut t, translation);
    if adj.has_adjust_transform {
        add_translate(&mut t, adj.pre_translation);
        mul_rotate_quat(&mut t, adj.pre_rotation);
        mul_scale_real(&mut t, adj.pre_scale);
        for x in t.translation.iter_mut() {
            *x *= adj.translation_scale;
        }
    }
    if adj.mirror_axis != 0 {
        mirror_translation(&mut t.translation, adj.mirror_axis);
        mirror_rotation(&mut t.rotation, adj.mirror_axis);
    }
    t
}

/// `ufbxi_get_geometry_transform`.
pub fn get_geometry_transform(p: &NodeProps, adj: &Adjust) -> Transform {
    let mut t = IDENTITY_TRANSFORM;
    mul_scale(&mut t, p.geometric_scaling);
    mul_rotate(&mut t, p.geometric_rotation, 0);
    add_translate(&mut t, p.geometric_translation);
    if adj.has_adjust_transform {
        for x in t.translation.iter_mut() {
            *x *= adj.translation_scale;
        }
    }
    if adj.mirror_axis != 0 {
        mirror_translation(&mut t.translation, adj.mirror_axis);
        mirror_rotation(&mut t.rotation, adj.mirror_axis);
    }
    t
}

/// `ufbxi_round_if_near` over `ufbxi_pow10_targets`.
pub fn round_if_near(value: f64) -> f64 {
    const TARGETS: [f64; 19] = [
        0.0, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6,
        1e7, 1e8, 1e9,
    ];
    for target in TARGETS {
        let mut error = target * 9.536_743_164_062_5e-7;
        if error < 0.0 {
            error = -error;
        }
        if error < 7.523_163_845_262_64e-37 {
            error = 7.523_163_845_262_64e-37;
        }
        if value >= target - error && value <= target + error {
            return target;
        }
    }
    value
}

/// Result of `ufbxi_update_node` for one node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeState {
    pub local: Transform,
    pub geometry: Transform,
    pub inherit_scale: Vec3,
    pub node_to_parent: Mat,
    pub node_to_world: Mat,
    pub geometry_to_node: Mat,
    pub geometry_to_world: Mat,
    pub unscaled_node_to_world: Mat,
    pub has_geometry_transform: bool,
}

/// `ufbxi_update_node` for a non-root node with `UFBX_INHERIT_MODE_NORMAL`
/// (the only effective mode under `UFBX_INHERIT_MODE_HANDLING_COMPENSATE`).
pub fn update_node(local: Transform, geometry: Transform, parent: Option<&NodeState>) -> NodeState {
    let node_to_parent = transform_to_matrix(&local);
    let unscaled_node_to_parent = unscaled_transform_to_matrix(&local);
    let (node_to_world, unscaled_node_to_world) = match parent {
        Some(p) => (
            matrix_mul(&p.node_to_world, &node_to_parent),
            matrix_mul(&p.node_to_world, &unscaled_node_to_parent),
        ),
        None => (node_to_parent, unscaled_node_to_parent),
    };
    let (geometry_to_node, geometry_to_world, has_geometry_transform) =
        if !is_transform_identity(&geometry) {
            let g = transform_to_matrix(&geometry);
            (g, matrix_mul(&node_to_world, &g), true)
        } else {
            (IDENTITY_MATRIX, node_to_world, false)
        };
    NodeState {
        local,
        geometry,
        inherit_scale: local.scale,
        node_to_parent,
        node_to_world,
        geometry_to_node,
        geometry_to_world,
        unscaled_node_to_world,
        has_geometry_transform,
    }
}

/// The synthetic root after `ufbxi_setup_root_node` without a root transform.
pub fn root_state() -> NodeState {
    update_node(IDENTITY_TRANSFORM, IDENTITY_TRANSFORM, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_quat_matrix_round_trip_matches_ufbx_bits() {
        // Pinned ufbx 0.23.1 engine-converted stream of a corpus file,
        // node 489354617: euler_rotation and local_transform.rotation bits.
        let q = euler_to_quat([f64::from_bits(0xc056_8000_2727_05d2), 0.0, 0.0], 0);
        assert_eq!(q[0].to_bits(), 0xbfe6_a09e_856b_e051);
        assert_eq!(q[3].to_bits(), 0x3fe6_a09e_4792_971e);
        let m = transform_to_matrix(&Transform {
            translation: [0.0; 3],
            rotation: q,
            scale: [1.0; 3],
        });
        let back = matrix_to_transform(&m);
        assert!((back.rotation[0] - q[0]).abs() < 1e-15);
        assert_eq!(
            axis_matrix([0, 4, 3], [0, 2, 4]).map(|m| matrix_determinant(&m)),
            Some(1.0)
        );
        assert!(axes_valid([0, 2, 4]) && !axes_valid([0, 0, 4]));
    }
}
