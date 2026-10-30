//! Experimental static material, texture, media, camera and light extraction.
//! Names and selected properties are preserved; full ufbx projection is pending.
//! Property names and defaults: <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
use crate::{
    scene::{ConnectionKind, Error, SceneIndex},
    syntax::{Document, Property},
};
use std::{borrow::Cow, mem::size_of};

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_objects: usize,
    pub max_embedded_bytes: usize,
    pub max_allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_objects: 1 << 20,
            max_embedded_bytes: 256 << 20,
            max_allocation: 512 << 20,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Binary,
    Ascii,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Number {
    pub value: f64,
    pub int_value: i64,
    pub authored: bool,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub value: [f64; 3],
    pub authored: bool,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialColor {
    pub value: [f64; 4],
    pub components: u8,
    pub authored: bool,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialProperties {
    pub diffuse_color: Option<MaterialColor>,
    pub diffuse_legacy: Option<MaterialColor>,
    pub diffuse_factor: Option<Number>,
    pub specular_color: Option<MaterialColor>,
    pub specular_legacy: Option<MaterialColor>,
    pub specular_factor: Option<Number>,
    pub emissive_color: Option<MaterialColor>,
    pub emissive_legacy: Option<MaterialColor>,
    pub emissive_factor: Option<Number>,
    pub transparency_factor: Option<Number>,
    pub transparent_factor: Option<Number>,
    pub transparent_color: Option<MaterialColor>,
    pub transparent_legacy: Option<MaterialColor>,
    pub shininess: Option<Number>,
    pub shininess_exponent: Option<Number>,
    pub base_color: Option<MaterialColor>,
    pub metalness: Option<Number>,
    pub roughness: Option<Number>,
}
#[derive(Debug)]
pub struct Material<'a> {
    pub shading_model: Option<&'a [u8]>,
    pub properties: MaterialProperties,
}
#[derive(Debug, Default)]
pub struct Texture<'a> {
    pub layered: bool,
    pub filename: Option<&'a [u8]>,
    pub relative_filename: Option<&'a [u8]>,
    pub uv_set: Option<&'a [u8]>,
}
#[derive(Debug, Default)]
pub struct Video<'a> {
    pub filename: Option<&'a [u8]>,
    pub relative_filename: Option<&'a [u8]>,
    pub content: Option<Cow<'a, [u8]>>,
}
#[derive(Debug, Clone, Default)]
pub struct Camera {
    pub projection: Option<Number>,
    pub field_of_view: Option<Number>,
    pub focal_length: Option<Number>,
    pub near_plane: Option<Number>,
    pub far_plane: Option<Number>,
    pub aspect_width: Option<Number>,
    pub aspect_height: Option<Number>,
}
#[derive(Debug, Clone, Default)]
pub struct Light {
    pub light_type: Option<Number>,
    pub color: Option<Vector>,
    pub intensity: Option<Number>,
    pub inner_angle: Option<Number>,
    pub hot_spot: Option<Number>,
    pub outer_angle: Option<Number>,
    pub cone_angle: Option<Number>,
    pub cone_angle_legacy: Option<Number>,
    pub cast_light: Option<Number>,
    pub cast_shadows: Option<Number>,
}
#[derive(Debug)]
// Inline variants keep all object storage in the fallibly reserved, budgeted vector.
#[allow(clippy::large_enum_variant)]
pub enum Data<'a> {
    Material(Material<'a>),
    Texture(Texture<'a>),
    Video(Video<'a>),
    Camera(Camera),
    Light(Light),
}
#[derive(Debug)]
pub struct Surface<'a> {
    pub object: usize,
    pub node: usize,
    pub data: Data<'a>,
}
#[derive(Debug)]
pub struct Surfaces<'a> {
    pub entries: Vec<Surface<'a>>,
    pub retained_bytes: usize,
    pub embedded_bytes: usize,
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
        Property::Bool(v) | Property::BoolAlt(v) => Some(f64::from(i8::from_ne_bytes([*v]))),
        Property::AsciiBare(v) => Some(crate::syntax::bare_number(v).1),
        Property::AsciiNegativeZero => Some(-0.0),
        _ => None,
    }
}
fn scalar(p: &[Property<'_>], authored: bool) -> Option<Number> {
    let property = p.get(4)?;
    let value = number(property)?;
    let int_value = match property {
        Property::I16(v) => i64::from(*v),
        Property::I32(v) => i64::from(*v),
        Property::I64(v) => *v,
        Property::AsciiBare(v) => crate::syntax::bare_number(v).0,
        _ => {
            if value.is_nan() {
                i64::MIN
            } else {
                crate::numeric::saturating_i64(value)
            }
        }
    };
    Some(Number {
        value,
        int_value,
        authored,
    })
}
fn vector(p: &[Property<'_>], authored: bool) -> Option<Vector> {
    Some(Vector {
        value: [number(p.get(4)?)?, number(p.get(5)?)?, number(p.get(6)?)?],
        authored,
    })
}
fn material_color(p: &[Property<'_>], authored: bool) -> Option<MaterialColor> {
    let values = p.get(4..)?;
    let mut value = [0.0; 4];
    let mut count = 0;
    for (dst, src) in value.iter_mut().zip(values.iter()) {
        let Some(v) = number(src) else { break };
        *dst = v;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some(MaterialColor {
        value,
        components: u8::try_from(count).unwrap(),
        authored,
    })
}
fn apply_material(
    m: &mut MaterialProperties,
    name: &[u8],
    p: &[Property<'_>],
    authored: bool,
    node: usize,
) -> Result<(), Error> {
    let invalid = || Error::InvalidSurface { node };
    match name {
        b"Diffuse" => m.diffuse_legacy = Some(material_color(p, authored).ok_or_else(invalid)?),
        b"DiffuseColor" => m.diffuse_color = Some(material_color(p, authored).ok_or_else(invalid)?),
        b"DiffuseFactor" => m.diffuse_factor = Some(scalar(p, authored).ok_or_else(invalid)?),
        b"Specular" => m.specular_legacy = Some(material_color(p, authored).ok_or_else(invalid)?),
        b"SpecularColor" => {
            m.specular_color = Some(material_color(p, authored).ok_or_else(invalid)?)
        }
        b"SpecularFactor" => m.specular_factor = Some(scalar(p, authored).ok_or_else(invalid)?),
        b"Emissive" => m.emissive_legacy = Some(material_color(p, authored).ok_or_else(invalid)?),
        b"EmissiveColor" => {
            m.emissive_color = Some(material_color(p, authored).ok_or_else(invalid)?)
        }
        b"EmissiveFactor" => m.emissive_factor = Some(scalar(p, authored).ok_or_else(invalid)?),
        b"Transparent" => {
            m.transparent_legacy = Some(material_color(p, authored).ok_or_else(invalid)?)
        }
        b"TransparentColor" => {
            m.transparent_color = Some(material_color(p, authored).ok_or_else(invalid)?)
        }
        b"TransparentFactor" => {
            m.transparent_factor = Some(scalar(p, authored).ok_or_else(invalid)?)
        }
        b"TransparencyFactor" => {
            m.transparency_factor = Some(scalar(p, authored).ok_or_else(invalid)?)
        }
        b"Shininess" => m.shininess = Some(scalar(p, authored).ok_or_else(invalid)?),
        b"ShininessExponent" => {
            m.shininess_exponent = Some(scalar(p, authored).ok_or_else(invalid)?)
        }
        b"BaseColor" => m.base_color = Some(material_color(p, authored).ok_or_else(invalid)?),
        b"Metalness" => m.metalness = Some(scalar(p, authored).ok_or_else(invalid)?),
        b"Roughness" => m.roughness = Some(scalar(p, authored).ok_or_else(invalid)?),
        _ => (),
    }
    Ok(())
}
fn apply_camera(
    c: &mut Camera,
    name: &[u8],
    p: &[Property<'_>],
    authored: bool,
    node: usize,
) -> Result<(), Error> {
    let dst = match name {
        b"CameraProjectionType" => Some(&mut c.projection),
        b"FieldOfView" => Some(&mut c.field_of_view),
        b"FocalLength" => Some(&mut c.focal_length),
        b"NearPlane" => Some(&mut c.near_plane),
        b"FarPlane" => Some(&mut c.far_plane),
        b"AspectWidth" => Some(&mut c.aspect_width),
        b"AspectHeight" => Some(&mut c.aspect_height),
        _ => None,
    };
    if let Some(dst) = dst {
        *dst = Some(scalar(p, authored).ok_or(Error::InvalidSurface { node })?)
    }
    Ok(())
}
fn apply_light(
    l: &mut Light,
    name: &[u8],
    p: &[Property<'_>],
    authored: bool,
    node: usize,
) -> Result<(), Error> {
    if name == b"Color" {
        l.color = Some(vector(p, authored).ok_or(Error::InvalidSurface { node })?);
        return Ok(());
    }
    let dst = match name {
        b"LightType" => Some(&mut l.light_type),
        b"Intensity" => Some(&mut l.intensity),
        b"InnerAngle" => Some(&mut l.inner_angle),
        b"HotSpot" => Some(&mut l.hot_spot),
        b"OuterAngle" => Some(&mut l.outer_angle),
        b"ConeAngle" => Some(&mut l.cone_angle),
        b"Cone angle" => Some(&mut l.cone_angle_legacy),
        b"CastLight" => Some(&mut l.cast_light),
        b"CastShadows" => Some(&mut l.cast_shadows),
        _ => None,
    };
    if let Some(dst) = dst {
        *dst = Some(scalar(p, authored).ok_or(Error::InvalidSurface { node })?)
    }
    Ok(())
}
fn b64val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
fn decoded_len(data: &[u8]) -> Option<usize> {
    if !data.len().is_multiple_of(4) {
        return None;
    }
    let mut size = data.len() / 4 * 3;
    if let Some(q) = data.as_chunks::<4>().0.last() {
        if q[3] == b'=' {
            size -= 1;
            if q[2] == b'=' {
                size -= 1;
            }
        }
    }
    for (i, q) in data.as_chunks::<4>().0.iter().enumerate() {
        let last = i + 1 == data.len() / 4;
        if b64val(q[0]).is_none()
            || b64val(q[1]).is_none()
            || (!(q[2] == b'=' && last) && b64val(q[2]).is_none())
            || (!(q[3] == b'=' && last) && b64val(q[3]).is_none())
            || (q[2] == b'=' && q[3] != b'=')
        {
            return None;
        }
    }
    Some(size)
}
fn decode_into(src: &[u8], out: &mut Vec<u8>) -> Option<()> {
    decoded_len(src)?;
    for q in src.as_chunks::<4>().0 {
        let a = b64val(q[0])?;
        let b = b64val(q[1])?;
        let c = if q[2] == b'=' { 0 } else { b64val(q[2])? };
        let d = if q[3] == b'=' { 0 } else { b64val(q[3])? };
        out.push((a << 2) | (b >> 4));
        if q[2] != b'=' {
            out.push((b << 4) | (c >> 2));
        }
        if q[3] != b'=' {
            out.push((c << 6) | d);
        }
    }
    Some(())
}
fn content<'a>(
    p: &'a [Property<'a>],
    encoding: Encoding,
    used: &mut usize,
    embedded: &mut usize,
    limits: &Limits,
    node: usize,
) -> Result<Cow<'a, [u8]>, Error> {
    let invalid = || Error::InvalidEmbedded { node };
    if encoding == Encoding::Binary && p.len() == 1 {
        let data = match &p[0] {
            Property::Raw(v) => *v,
            Property::String(v) => v.as_ref(),
            _ => return Err(invalid()),
        };
        *embedded = embedded
            .checked_add(data.len())
            .ok_or(Error::LimitExceeded("embedded bytes"))?;
        if *embedded > limits.max_embedded_bytes {
            return Err(Error::LimitExceeded("embedded bytes"));
        }
        return Ok(Cow::Borrowed(data));
    }
    let mut total = 0usize;
    for part in p {
        let data = match part {
            Property::Raw(v) => *v,
            Property::String(v) => v.as_ref(),
            _ => return Err(invalid()),
        };
        let size = if encoding == Encoding::Ascii {
            decoded_len(data).ok_or_else(invalid)?
        } else {
            data.len()
        };
        total = total
            .checked_add(size)
            .ok_or(Error::LimitExceeded("embedded bytes"))?;
    }
    *embedded = embedded
        .checked_add(total)
        .ok_or(Error::LimitExceeded("embedded bytes"))?;
    if *embedded > limits.max_embedded_bytes {
        return Err(Error::LimitExceeded("embedded bytes"));
    }
    let mut out = Vec::new();
    crate::scene::reserve(&mut out, total, used, limits.max_allocation)?;
    for part in p {
        let data = match part {
            Property::Raw(v) => *v,
            Property::String(v) => v.as_ref(),
            _ => return Err(invalid()),
        };
        if encoding == Encoding::Ascii {
            decode_into(data, &mut out).ok_or_else(invalid)?
        } else {
            out.extend_from_slice(data)
        }
    }
    Ok(Cow::Owned(out))
}
type Template<'a> = Option<(usize, Option<&'a [u8]>)>;
fn first_template<'a>(doc: &'a Document<'a>, kind: &[u8]) -> Result<Template<'a>, Error> {
    for (i, n) in doc.nodes.iter().enumerate() {
        if n.name != b"ObjectType" {
            continue;
        }
        let Some(parent) = n.parent.and_then(|p| doc.nodes.get(p)) else {
            continue;
        };
        if parent.name != b"Definitions" || parent.parent.is_some() {
            continue;
        }
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        if p.first().and_then(bytes) != Some(kind) {
            continue;
        }
        let Some((j, t)) = doc
            .nodes
            .iter()
            .enumerate()
            .find(|(_, c)| c.parent == Some(i) && c.name == b"PropertyTemplate")
        else {
            return Ok(None);
        };
        let subtype = doc
            .properties
            .get(t.properties.clone())
            .ok_or(Error::InvalidDom)?
            .first()
            .and_then(bytes)
            .map(crate::scene::fbx_subtype);
        return Ok(Some((j, subtype)));
    }
    Ok(None)
}
/// Extract direct objects and selected Properties70 values. Material template
/// defaults are used; derived shader maps and full camera projection are pending.
pub fn extract<'a>(
    doc: &'a Document<'a>,
    index: &SceneIndex<'a>,
    encoding: Encoding,
    limits: &Limits,
) -> Result<Surfaces<'a>, Error> {
    let mut count = 0usize;
    for obj in &index.objects {
        let n = doc.nodes.get(obj.node).ok_or(Error::InvalidDom)?;
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        let subtype = p.get(2).and_then(bytes).map(crate::scene::fbx_subtype);
        if matches!(
            n.name,
            b"Material" | b"Texture" | b"LayeredTexture" | b"Video"
        ) || (n.name == b"NodeAttribute" && matches!(subtype, Some(b"Camera" | b"Light")))
        {
            count += 1;
        }
    }
    if count > limits.max_objects {
        return Err(Error::LimitExceeded("surface objects"));
    }
    let requested = count
        .checked_mul(size_of::<Surface<'_>>())
        .ok_or(Error::LimitExceeded("allocation"))?;
    if requested > limits.max_allocation {
        return Err(Error::LimitExceeded("allocation"));
    }
    let mut entries = Vec::new();
    let mut used = 0;
    crate::scene::reserve(&mut entries, count, &mut used, limits.max_allocation)?;
    let mut material_default = MaterialProperties::default();
    let mut template = None;
    for (i, n) in doc.nodes.iter().enumerate() {
        if n.name != b"ObjectType" {
            continue;
        }
        let Some(parent) = n.parent.and_then(|p| doc.nodes.get(p)) else {
            continue;
        };
        if parent.name != b"Definitions" || parent.parent.is_some() {
            continue;
        }
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        if p.first().and_then(bytes) != Some(b"Material") {
            continue;
        }
        template = doc
            .nodes
            .iter()
            .enumerate()
            .find(|(_, c)| c.parent == Some(i) && c.name == b"PropertyTemplate")
            .map(|(j, _)| j);
        break;
    }
    if let Some(template) = template {
        for (i, n) in doc.nodes.iter().enumerate() {
            if n.name != b"P" {
                continue;
            }
            let Some(pnode) = n.parent.and_then(|p| doc.nodes.get(p)) else {
                continue;
            };
            if pnode.name != b"Properties70" || pnode.parent != Some(template) {
                continue;
            }
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            if let Some(name) = p.first().and_then(bytes) {
                apply_material(&mut material_default, name, p, false, i)?;
            }
        }
    }
    let mut camera_default = Camera::default();
    let mut light_default = Light::default();
    if let Some((template, subtype)) = first_template(doc, b"NodeAttribute")? {
        for (i, n) in doc.nodes.iter().enumerate() {
            if n.name != b"P" {
                continue;
            }
            let Some(parent) = n.parent.and_then(|p| doc.nodes.get(p)) else {
                continue;
            };
            if parent.name != b"Properties70" || parent.parent != Some(template) {
                continue;
            }
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            let Some(name) = p.first().and_then(bytes) else {
                continue;
            };
            match subtype {
                Some(b"Camera") => apply_camera(&mut camera_default, name, p, false, i)?,
                Some(b"Light") => apply_light(&mut light_default, name, p, false, i)?,
                _ => (),
            }
        }
    }
    let mut texture_template_subtype = None;
    let mut texture_template_uv = None;
    if let Some((template, subtype)) = first_template(doc, b"Texture")? {
        texture_template_subtype = subtype;
        for (i, n) in doc.nodes.iter().enumerate() {
            if n.name != b"P" {
                continue;
            }
            let Some(parent) = n.parent.and_then(|p| doc.nodes.get(p)) else {
                continue;
            };
            if parent.name != b"Properties70" || parent.parent != Some(template) {
                continue;
            }
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            if p.first().and_then(bytes) == Some(b"UVSet") {
                texture_template_uv = Some(
                    p.get(4)
                        .and_then(bytes)
                        .ok_or(Error::InvalidSurface { node: i })?,
                );
            }
        }
    }
    for (object, obj) in index.objects.iter().enumerate() {
        let n = doc.nodes.get(obj.node).ok_or(Error::InvalidDom)?;
        let p = doc
            .properties
            .get(n.properties.clone())
            .ok_or(Error::InvalidDom)?;
        let data = match (
            n.name,
            p.get(2).and_then(bytes).map(crate::scene::fbx_subtype),
        ) {
            (b"Material", _) => Some(Data::Material(Material {
                shading_model: None,
                properties: material_default.clone(),
            })),
            (b"Texture", subtype) => Some(Data::Texture(Texture {
                uv_set: if subtype == texture_template_subtype {
                    texture_template_uv
                } else {
                    None
                },
                ..Default::default()
            })),
            (b"LayeredTexture", _) => Some(Data::Texture(Texture {
                layered: true,
                ..Default::default()
            })),
            (b"Video", _) => Some(Data::Video(Video::default())),
            (b"NodeAttribute", Some(b"Camera")) => Some(Data::Camera(camera_default.clone())),
            (b"NodeAttribute", Some(b"Light")) => Some(Data::Light(light_default.clone())),
            _ => None,
        };
        if let Some(data) = data {
            entries.push(Surface {
                object,
                node: obj.node,
                data,
            });
        }
    }
    let mut embedded = 0usize;
    for (i, n) in doc.nodes.iter().enumerate() {
        let Some(parent) = n.parent else { continue };
        if let Ok(si) = entries.binary_search_by_key(&parent, |s| s.node) {
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            match (&mut entries[si].data, n.name) {
                (Data::Material(m), b"ShadingModel") => {
                    m.shading_model = Some(
                        p.first()
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                (Data::Texture(t), b"FileName" | b"Filename") => {
                    t.filename = Some(
                        p.first()
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                (Data::Texture(t), b"RelativeFileName" | b"RelativeFilename") => {
                    t.relative_filename = Some(
                        p.first()
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                (Data::Video(v), b"FileName" | b"Filename") => {
                    v.filename = Some(
                        p.first()
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                (Data::Video(v), b"RelativeFileName" | b"RelativeFilename") => {
                    v.relative_filename = Some(
                        p.first()
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                (Data::Video(v), b"Content") => {
                    v.content = Some(content(p, encoding, &mut used, &mut embedded, limits, i)?)
                }
                _ => (),
            }
        } else if n.name == b"P" {
            let Some(parent_node) = doc.nodes.get(parent) else {
                return Err(Error::InvalidDom);
            };
            if parent_node.name != b"Properties70" {
                continue;
            }
            let Some(owner) = parent_node.parent else {
                continue;
            };
            let Ok(si) = entries.binary_search_by_key(&owner, |s| s.node) else {
                continue;
            };
            let p = doc
                .properties
                .get(n.properties.clone())
                .ok_or(Error::InvalidDom)?;
            let Some(name) = p.first().and_then(bytes) else {
                continue;
            };
            match &mut entries[si].data {
                Data::Material(m) => apply_material(&mut m.properties, name, p, true, i)?,
                Data::Camera(c) => apply_camera(c, name, p, true, i)?,
                Data::Light(l) => apply_light(l, name, p, true, i)?,
                Data::Texture(t) if name == b"UVSet" => {
                    t.uv_set = Some(
                        p.get(4)
                            .and_then(bytes)
                            .ok_or(Error::InvalidSurface { node: i })?,
                    )
                }
                _ => (),
            }
        }
    }
    Ok(Surfaces {
        entries,
        retained_bytes: used,
        embedded_bytes: embedded,
    })
}
impl<'doc> Surfaces<'doc> {
    /// Texture connections to a material, preserving FBX connection order and property names.
    pub fn material_textures<'a>(
        &'a self,
        material: usize,
        index: &'a SceneIndex<'_>,
    ) -> impl Iterator<Item = (usize, &'a [u8])> + 'a {
        let object = self.entries.get(material).and_then(|s| {
            if matches!(s.data, Data::Material(_)) {
                Some(s.object)
            } else {
                None
            }
        });
        index
            .connections
            .iter()
            .filter_map(move |c| match (&c.kind, c.destination) {
                (ConnectionKind::Property(name), Some(d)) if Some(d) == object => self
                    .entries
                    .binary_search_by_key(&c.source, |s| s.object)
                    .ok()
                    .and_then(|i| self.entries.get(i))
                    .filter(|s| matches!(s.data, Data::Texture(_)))
                    .map(|s| (s.object, *name)),
                _ => None,
            })
    }
    /// A video connected to a texture; the result shares the video's content bytes.
    pub fn texture_video<'a>(
        &'a self,
        texture: usize,
        index: &SceneIndex<'_>,
    ) -> Option<&'a Video<'doc>> {
        let object = self.entries.get(texture).and_then(|s| {
            if matches!(s.data, Data::Texture(_)) {
                Some(s.object)
            } else {
                None
            }
        })?;
        for c in &index.connections {
            if c.kind == ConnectionKind::Object && c.destination == Some(object) {
                if let Some(Data::Video(v)) = self
                    .entries
                    .binary_search_by_key(&c.source, |s| s.object)
                    .ok()
                    .and_then(|i| self.entries.get(i))
                    .map(|s| &s.data)
                {
                    return Some(v);
                }
            }
        }
        None
    }
}

/// Selected ufbx-style Lambert/Phong maps. `has_value` distinguishes an
/// authored/template value from a derived factor default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarMap {
    pub value: f64,
    pub has_value: bool,
    pub texture: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorMap {
    pub value: [f64; 4],
    pub has_value: bool,
    pub texture: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicProjection {
    pub base_color: ColorMap,
    pub base_factor: ScalarMap,
    pub specular_color: ColorMap,
    pub specular_factor: ScalarMap,
    pub roughness: ScalarMap,
    pub emission_color: ColorMap,
    pub emission_factor: ScalarMap,
    pub transmission_color: ColorMap,
    pub transmission_factor: ScalarMap,
    pub normal_texture: Option<usize>,
}
fn color_map(value: Option<MaterialColor>, texture: Option<usize>) -> ColorMap {
    match value {
        Some(v) => ColorMap {
            value: [
                v.value[0],
                v.value[1],
                v.value[2],
                if v.components == 4 { v.value[3] } else { 1.0 },
            ],
            has_value: true,
            texture,
        },
        None => ColorMap {
            value: [0.0; 4],
            has_value: false,
            texture,
        },
    }
}
fn factor_map(value: Option<Number>, color: ColorMap, texture: Option<usize>) -> ScalarMap {
    match value {
        Some(v) => ScalarMap {
            value: v.value,
            has_value: true,
            texture,
        },
        None => ScalarMap {
            value: if color.has_value && color.value.iter().any(|v| *v != 0.0) {
                1.0
            } else {
                0.0
            },
            has_value: false,
            texture,
        },
    }
}
impl Surfaces<'_> {
    fn texture_for(
        &self,
        material: usize,
        index: &SceneIndex<'_>,
        names: &[&[u8]],
    ) -> Result<Option<usize>, Error> {
        let mut result = None;
        for &name in names {
            let mut found = None;
            for (texture, property) in self.material_textures(material, index) {
                if property == name {
                    if found.is_some_and(|v| v != texture) {
                        return Err(Error::AmbiguousTextureBinding);
                    }
                    found = Some(texture);
                }
            }
            if found.is_some() {
                result = found;
            }
        }
        Ok(result)
    }
    /// Project ordinary Lambert/Phong material maps. Explicit shader bindings and
    /// other shading models return an error rather than an approximate map.
    /// Map order and shininess transform follow ufbx 0.23.1
    /// `ufbxi_fbx_*_shader_pbr_mapping` and `ufbxi_update_factor` (MIT).
    /// <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
    pub fn basic_projection(
        &self,
        material: usize,
        index: &SceneIndex<'_>,
        doc: &Document<'_>,
    ) -> Result<BasicProjection, Error> {
        let entry = self.entries.get(material).ok_or(Error::InvalidDom)?;
        let Data::Material(m) = &entry.data else {
            return Err(Error::InvalidSurface { node: entry.node });
        };
        let phong = match m.shading_model {
            Some(b"phong" | b"Phong") => true,
            Some(b"lambert" | b"Lambert") => false,
            _ => return Err(Error::UnsupportedShader),
        };
        for c in &index.connections {
            if c.kind == ConnectionKind::Object && c.destination == Some(entry.object) {
                let name = index
                    .objects
                    .get(c.source)
                    .and_then(|o| doc.nodes.get(o.node))
                    .map(|n| n.name)
                    .ok_or(Error::InvalidDom)?;
                if name == b"Implementation" {
                    return Err(Error::UnsupportedShader);
                }
            }
        }
        let p = &m.properties;
        let base = color_map(
            p.diffuse_color.or(p.diffuse_legacy),
            self.texture_for(material, index, &[b"Diffuse", b"DiffuseColor"])?,
        );
        let spec = if phong {
            color_map(
                p.specular_color.or(p.specular_legacy),
                self.texture_for(material, index, &[b"Specular", b"SpecularColor"])?,
            )
        } else {
            color_map(None, None)
        };
        let emission = color_map(
            p.emissive_color.or(p.emissive_legacy),
            self.texture_for(material, index, &[b"Emissive", b"EmissiveColor"])?,
        );
        let transmission = color_map(
            p.transparent_color.or(p.transparent_legacy),
            self.texture_for(material, index, &[b"Transparent", b"TransparentColor"])?,
        );
        let roughness = if phong {
            p.shininess_exponent.or(p.shininess).map(|v| {
                let n = v.value;
                let r = if n >= 0.0 { 1.0 - n.sqrt() * 0.1 } else { n };
                if r >= 0.0 {
                    r
                } else {
                    0.0
                }
            })
        } else {
            None
        };
        Ok(BasicProjection {
            base_color: base,
            base_factor: factor_map(
                p.diffuse_factor,
                base,
                self.texture_for(material, index, &[b"DiffuseFactor"])?,
            ),
            specular_color: spec,
            specular_factor: factor_map(
                if phong { p.specular_factor } else { None },
                spec,
                if phong {
                    self.texture_for(material, index, &[b"SpecularFactor"])?
                } else {
                    None
                },
            ),
            roughness: ScalarMap {
                value: roughness.unwrap_or(0.0),
                has_value: roughness.is_some(),
                texture: if phong {
                    self.texture_for(material, index, &[b"Shininess", b"ShininessExponent"])?
                } else {
                    None
                },
            },
            emission_color: emission,
            emission_factor: factor_map(
                p.emissive_factor,
                emission,
                self.texture_for(material, index, &[b"EmissiveFactor"])?,
            ),
            transmission_color: transmission,
            transmission_factor: factor_map(
                p.transparency_factor.or(p.transparent_factor),
                transmission,
                self.texture_for(
                    material,
                    index,
                    &[b"TransparentFactor", b"TransparencyFactor"],
                )?,
            ),
            normal_texture: self.texture_for(material, index, &[b"NormalMap"])?,
        })
    }
}

/// Basic static light values in the ufbx convention, before node-axis adjustments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightProjection {
    pub light_type: i64,
    pub color: [f64; 3],
    pub intensity: f64,
    pub inner_angle: f64,
    pub outer_angle: f64,
    pub cast_light: bool,
    pub cast_shadows: bool,
}
impl Light {
    /// Default values and alias precedence follow ufbx 0.23.1 `ufbxi_update_light`.
    pub fn basic_projection(&self) -> LightProjection {
        let ty = self.light_type.map_or(0, |v| v.int_value);
        LightProjection {
            light_type: if (0..=4).contains(&ty) { ty } else { 0 },
            color: self.color.map_or([1.0; 3], |v| v.value),
            intensity: self.intensity.map_or(100.0, |v| v.value) / 100.0,
            inner_angle: self.inner_angle.or(self.hot_spot).map_or(0.0, |v| v.value),
            outer_angle: self
                .outer_angle
                .or(self.cone_angle)
                .or(self.cone_angle_legacy)
                .map_or(0.0, |v| v.value),
            cast_light: self.cast_light.is_none_or(|v| v.int_value != 0),
            cast_shadows: self.cast_shadows.is_some_and(|v| v.int_value != 0),
        }
    }
}
/// Selected camera values. FOV, film gate, resolution and node orientation
/// require the remaining ufbx camera projection logic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraProjection {
    pub projection: i64,
    pub focal_length_mm: f64,
    pub near_plane: f64,
    pub far_plane: f64,
}
impl Camera {
    /// `geometry_scale` is the already established ufbx profile scale.
    pub fn basic_projection(&self, geometry_scale: f64) -> CameraProjection {
        let mode = self.projection.map_or(0, |v| v.int_value);
        CameraProjection {
            projection: if (0..=1).contains(&mode) { mode } else { 0 },
            focal_length_mm: self.focal_length.map_or(0.0, |v| v.value),
            near_plane: self.near_plane.map_or(0.0, |v| v.value) * geometry_scale,
            far_plane: self.far_plane.map_or(0.0, |v| v.value) * geometry_scale,
        }
    }
}
