//! Light, camera and bone attributes, ported operation for operation from
//! pinned ufbx 0.23.1 (MIT, <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>):
//! `ufbxi_update_light`, `ufbxi_update_camera` (with the
//! `ufbxi_aperture_formats` table), `ufbxi_update_bone` and `ufbxi_read_bone`,
//! plus the attribute part of `ufbxi_update_adjust_transforms` (light
//! `local_direction`, camera `projection_axes`). `ufbx_real` is `double`;
//! single-precision literals in ufbx.c (`0.5f`, `2.0f`) are exact in `f64`.
use crate::canon::{Obj, V};
use crate::scene_node as sn;
use crate::scene_property::PropertyLayers;

/// `ufbxi_find_real`: `value_real` of the property, or `def`.
fn real(p: &PropertyLayers<'_>, name: &[u8], def: f64) -> f64 {
    p.find(name).map_or(def, |p| p.value_real[0])
}

/// `ufbxi_find_int`.
fn int(p: &PropertyLayers<'_>, name: &[u8], def: i64) -> i64 {
    p.find(name).map_or(def, |p| p.value_int)
}

/// `ufbxi_find_enum`: out-of-range values fall back to `def`.
fn enumv(p: &PropertyLayers<'_>, name: &[u8], def: i64, max: i64) -> i64 {
    match p.find(name) {
        Some(p) if (0..=max).contains(&p.value_int) => p.value_int,
        Some(_) | None => def,
    }
}

/// `ufbxi_find_vec3`.
fn vec3(p: &PropertyLayers<'_>, name: &[u8], def: [f64; 3]) -> [f64; 3] {
    p.find(name)
        .map_or(def, |p| [p.value_real[0], p.value_real[1], p.value_real[2]])
}

/// Scene-level inputs of the attribute updates.
#[derive(Debug, Clone, Copy)]
pub struct SceneUnits {
    /// `metadata.geometry_scale`.
    pub geometry_scale: f64,
    /// `metadata.ortho_size_unit` (final value after `ufbxi_postprocess_scene`).
    pub ortho_size_unit: f64,
    /// `metadata.bone_prop_size_unit`.
    pub bone_prop_size_unit: f64,
    /// `metadata.bone_prop_limb_length_relative`.
    pub bone_prop_limb_length_relative: bool,
}

/// `ufbx_light` values set by `ufbxi_update_light`.
#[derive(Debug, Clone, Copy)]
pub struct Light {
    pub color: [f64; 3],
    pub intensity: f64,
    pub local_direction: [f64; 3],
    pub ty: i64,
    pub decay: i64,
    pub area_shape: i64,
    pub inner_angle: f64,
    pub outer_angle: f64,
    pub cast_light: bool,
    pub cast_shadows: bool,
}

impl Light {
    /// The canonical `ufbx_light` record fields.
    pub fn to_obj(self) -> Obj {
        Obj::new()
            .with("color", V::Vec(self.color.to_vec()))
            .with("intensity", V::F(self.intensity))
            .with("local_direction", V::Vec(self.local_direction.to_vec()))
            .with("type", V::I(self.ty))
            .with("decay", V::I(self.decay))
            .with("area_shape", V::I(self.area_shape))
            .with("inner_angle", V::F(self.inner_angle))
            .with("outer_angle", V::F(self.outer_angle))
            .with("cast_light", V::Bool(self.cast_light))
            .with("cast_shadows", V::Bool(self.cast_shadows))
    }
}

/// `ufbxi_update_light`. `local_direction` comes from
/// [`light_direction`] and is applied by the caller.
pub fn light(p: &PropertyLayers<'_>, local_direction: [f64; 3]) -> Light {
    let intensity = real(p, b"Intensity", 100.0) / 100.0;
    let color = vec3(p, b"Color", [1.0, 1.0, 1.0]);
    let ty = enumv(p, b"LightType", 0, 4);
    let decay = enumv(p, b"DecayType", 0, 3);
    let area_shape = enumv(p, b"AreaLightShape", 0, 1);
    let mut inner = real(p, b"HotSpot", 0.0);
    inner = real(p, b"InnerAngle", inner);
    let mut outer = real(p, b"Cone angle", 0.0);
    outer = real(p, b"ConeAngle", outer);
    outer = real(p, b"OuterAngle", outer);
    Light {
        color,
        intensity,
        local_direction,
        ty,
        decay,
        area_shape,
        inner_angle: inner,
        outer_angle: outer,
        cast_light: int(p, b"CastLight", 1) != 0,
        cast_shadows: int(p, b"CastShadows", 0) != 0,
    }
}

/// Light direction for single-attribute light nodes when the target light
/// axes differ from FBX's (`light_axes = { +X, -Z, +Y }`): the inverse axis
/// matrix applied to `(0, -1, 0)`. `None` keeps the default `(0, -1, 0)`.
pub fn light_direction(target_axes: [i64; 3]) -> Option<[f64; 3]> {
    if !sn::axes_valid(target_axes) {
        return None;
    }
    let m = sn::axis_matrix(target_axes, [0, 5, 2])?;
    let inv = sn::matrix_invert(&m);
    Some(sn::transform_direction(&inv, [0.0, -1.0, 0.0]))
}

/// Default `local_direction` of every light.
pub const DEFAULT_LIGHT_DIRECTION: [f64; 3] = [0.0, -1.0, 0.0];

/// Default `projection_axes` (zero-initialised `ufbx_coordinate_axes`).
pub const DEFAULT_CAMERA_AXES: [i64; 3] = [0, 0, 0];

/// `ufbxi_aperture_formats`: film size in 1/1000 inch.
const APERTURE_FORMATS: [(u16, u16); 12] = [
    (1000, 1000),
    (404, 295),
    (493, 292),
    (864, 630),
    (816, 612),
    (980, 735),
    (825, 446),
    (864, 732),
    (2066, 906),
    (1485, 991),
    (2080, 1480),
    (2772, 2072),
];

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;
const RAD_TO_DEG: f64 = 180.0 / std::f64::consts::PI;
const MM_TO_INCH: f64 = 0.0393700787;

/// `ufbxi_update_camera`.
pub fn camera(p: &PropertyLayers<'_>, units: &SceneUnits, projection_axes: [i64; 3]) -> Camera {
    let projection_mode = enumv(p, b"CameraProjectionType", 0, 1);
    let aspect_mode = enumv(p, b"AspectRatioMode", 0, 4);
    let aperture_mode = enumv(p, b"ApertureMode", 2, 3);
    let aperture_format = enumv(p, b"ApertureFormat", 0, 11);
    let gate_fit = enumv(p, b"GateFit", 0, 5);

    let mut near_plane = real(p, b"NearPlane", 0.0);
    let mut far_plane = real(p, b"FarPlane", 0.0);

    let mut aspect_x = real(p, b"AspectW", 0.0);
    let mut aspect_y = real(p, b"AspectH", 0.0);
    aspect_x = real(p, b"AspectWidth", aspect_x);
    aspect_y = real(p, b"AspectHeight", aspect_y);

    let fov = real(p, b"FieldOfView", 0.0);
    let fov_x = real(p, b"FieldOfViewX", 0.0);
    let fov_y = real(p, b"FieldOfViewY", 0.0);

    let focal_length = real(p, b"FocalLength", 0.0);
    let mut ortho_extent = units.ortho_size_unit * real(p, b"OrthoZoom", 1.0);

    let (fx, fy) = APERTURE_FORMATS[usize::try_from(aperture_format).unwrap()];
    let mut film = [fx as f64 * 0.001, fy as f64 * 0.001];
    let mut squeeze_ratio = if aperture_format == 7 { 2.0 } else { 1.0 };

    film[0] = real(p, b"FilmWidth", film[0]);
    film[1] = real(p, b"FilmHeight", film[1]);
    squeeze_ratio = real(p, b"FilmSqueezeRatio", squeeze_ratio);

    if aspect_x <= 0.0 && aspect_y <= 0.0 {
        aspect_x = if film[0] > 0.0 { film[0] } else { 1.0 };
        aspect_y = if film[1] > 0.0 { film[1] } else { 1.0 };
    } else if aspect_x <= 0.0 {
        if film[0] > 0.0 && film[1] > 0.0 {
            aspect_x = aspect_y / film[1] * film[0];
        } else {
            aspect_x = aspect_y;
        }
    } else if aspect_y <= 0.0 {
        if film[0] > 0.0 && film[1] > 0.0 {
            aspect_y = aspect_x / film[0] * film[1];
        } else {
            aspect_y = aspect_x;
        }
    }

    film[1] *= squeeze_ratio;

    ortho_extent *= units.geometry_scale;
    near_plane *= units.geometry_scale;
    far_plane *= units.geometry_scale;

    let (resolution_is_pixels, resolution) = match aspect_mode {
        // WINDOW_SIZE, FIXED_RATIO
        0 | 1 => (false, [aspect_x, aspect_y]),
        // FIXED_RESOLUTION
        2 => (true, [aspect_x, aspect_y]),
        // FIXED_WIDTH
        3 => (true, [aspect_x, aspect_x * aspect_y]),
        // FIXED_HEIGHT
        _ => (true, [aspect_y * aspect_x, aspect_y]),
    };

    let aspect_ratio = resolution[0] / resolution[1];
    let film_ratio = film[0] / film[1];

    // NONE 0, VERTICAL 1, HORIZONTAL 2, FILL 3, OVERSCAN 4, STRETCH 5.
    let mut fit = gate_fit;
    if fit == 3 {
        fit = if aspect_ratio > film_ratio { 2 } else { 1 };
    } else if fit == 4 {
        fit = if aspect_ratio < film_ratio { 2 } else { 1 };
    }
    let (aperture, ortho_size) = match fit {
        1 => (
            [film[1] * aspect_ratio, film[1]],
            [ortho_extent * aspect_ratio, ortho_extent],
        ),
        2 => (
            [film[0], film[0] / aspect_ratio],
            [ortho_extent, ortho_extent / aspect_ratio],
        ),
        _ => (film, [ortho_extent, ortho_extent]),
    };

    // HORIZONTAL_AND_VERTICAL 0, HORIZONTAL 1, VERTICAL 2, FOCAL_LENGTH 3.
    let (fov_deg, fov_tan) = match aperture_mode {
        0 => (
            [fov_x, fov_y],
            [
                (fov_x * (DEG_TO_RAD * 0.5)).tan(),
                (fov_y * (DEG_TO_RAD * 0.5)).tan(),
            ],
        ),
        1 => {
            let tx = (fov * (DEG_TO_RAD * 0.5)).tan();
            let ty = tx / aspect_ratio;
            ([fov, ty.atan() * RAD_TO_DEG * 2.0], [tx, ty])
        }
        2 => {
            let ty = (fov * (DEG_TO_RAD * 0.5)).tan();
            let tx = ty * aspect_ratio;
            ([tx.atan() * RAD_TO_DEG * 2.0, fov], [tx, ty])
        }
        _ => {
            let tx = aperture[0] / (focal_length * MM_TO_INCH) * 0.5;
            let ty = aperture[1] / (focal_length * MM_TO_INCH) * 0.5;
            (
                [tx.atan() * RAD_TO_DEG * 2.0, ty.atan() * RAD_TO_DEG * 2.0],
                [tx, ty],
            )
        }
    };
    let projection_plane = if projection_mode == 0 {
        fov_tan
    } else {
        ortho_size
    };
    Camera {
        projection_mode,
        resolution_is_pixels,
        resolution,
        field_of_view_deg: fov_deg,
        field_of_view_tan: fov_tan,
        orthographic_extent: ortho_extent,
        orthographic_size: ortho_size,
        projection_plane,
        aspect_ratio,
        near_plane,
        far_plane,
        projection_axes,
        aspect_mode,
        aperture_mode,
        gate_fit,
        aperture_format,
        focal_length_mm: focal_length,
        film_size_inch: film,
        aperture_size_inch: aperture,
        squeeze_ratio,
    }
}

/// `ufbx_camera` values set by `ufbxi_update_camera`.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub projection_mode: i64,
    pub resolution_is_pixels: bool,
    pub resolution: [f64; 2],
    pub field_of_view_deg: [f64; 2],
    pub field_of_view_tan: [f64; 2],
    pub orthographic_extent: f64,
    pub orthographic_size: [f64; 2],
    pub projection_plane: [f64; 2],
    pub aspect_ratio: f64,
    pub near_plane: f64,
    pub far_plane: f64,
    pub projection_axes: [i64; 3],
    pub aspect_mode: i64,
    pub aperture_mode: i64,
    pub gate_fit: i64,
    pub aperture_format: i64,
    pub focal_length_mm: f64,
    pub film_size_inch: [f64; 2],
    pub aperture_size_inch: [f64; 2],
    pub squeeze_ratio: f64,
}

impl Camera {
    /// The canonical `ufbx_camera` record fields.
    pub fn to_obj(self) -> Obj {
        let axes = Obj::new()
            .with("right", V::I(self.projection_axes[0]))
            .with("up", V::I(self.projection_axes[1]))
            .with("front", V::I(self.projection_axes[2]));
        Obj::new()
            .with("projection_mode", V::I(self.projection_mode))
            .with("resolution_is_pixels", V::Bool(self.resolution_is_pixels))
            .with("resolution", V::Vec(self.resolution.to_vec()))
            .with("field_of_view_deg", V::Vec(self.field_of_view_deg.to_vec()))
            .with("field_of_view_tan", V::Vec(self.field_of_view_tan.to_vec()))
            .with("orthographic_extent", V::F(self.orthographic_extent))
            .with("orthographic_size", V::Vec(self.orthographic_size.to_vec()))
            .with("projection_plane", V::Vec(self.projection_plane.to_vec()))
            .with("aspect_ratio", V::F(self.aspect_ratio))
            .with("near_plane", V::F(self.near_plane))
            .with("far_plane", V::F(self.far_plane))
            .with("projection_axes", V::Obj(axes))
            .with("aspect_mode", V::I(self.aspect_mode))
            .with("aperture_mode", V::I(self.aperture_mode))
            .with("gate_fit", V::I(self.gate_fit))
            .with("aperture_format", V::I(self.aperture_format))
            .with("focal_length_mm", V::F(self.focal_length_mm))
            .with("film_size_inch", V::Vec(self.film_size_inch.to_vec()))
            .with(
                "aperture_size_inch",
                V::Vec(self.aperture_size_inch.to_vec()),
            )
            .with("squeeze_ratio", V::F(self.squeeze_ratio))
    }
}

/// `ufbxi_update_bone`; `is_root` is `ufbxi_read_bone`'s `Root` sub-type.
pub(crate) fn bone(
    p: &PropertyLayers<'_>,
    units: &SceneUnits,
    is_root: bool,
    a: &crate::api_alloc::Budget,
) -> Result<Obj, crate::Error> {
    let unit = units.bone_prop_size_unit;
    let relative_length = if units.bone_prop_limb_length_relative {
        real(p, b"LimbLength", 1.0)
    } else {
        1.0
    };
    Obj::new()
        .try_with("radius", V::F(real(p, b"Size", unit) / unit), a)?
        .try_with("relative_length", V::F(relative_length), a)?
        .try_with("is_root", V::Bool(is_root), a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_direction_for_y_up_target() {
        // Target right-handed Y-up: FBX light axes {+X,-Z,+Y} map -Y to -Z.
        let d = light_direction([0, 2, 4]).unwrap_or(DEFAULT_LIGHT_DIRECTION);
        assert_eq!(d.map(f64::to_bits), [0.0f64, 0.0, -1.0].map(f64::to_bits));
    }
}
