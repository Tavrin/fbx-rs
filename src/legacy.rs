//! FBX 5.8/6.x scene construction, following ufbx 0.23.1 (MIT):
//! `ufbxi_read_object`, `ufbxi_read_synthetic_attribute`, `ufbxi_read_take*`.
use crate::api_alloc::Budget;
use crate::canon::Obj;
use crate::canon_scene::{DumpLimits, Element, ElementType as T};
use crate::scene_property::{AuthoredProperty, PropertyIndex, PropertyLayers, Type};
use crate::syntax::{Document, Property};
use crate::Error;

fn values<'a>(doc: &'a Document<'a>, n: usize) -> &'a [Property<'a>] {
    &doc.properties[doc.nodes[n].properties.clone()]
}
pub(crate) fn string<'a>(p: Option<&'a Property<'_>>) -> &'a [u8] {
    match p {
        Some(Property::String(v)) => v,
        Some(Property::AsciiBare(v)) => v,
        _ => b"",
    }
}
fn name(raw: &[u8]) -> &[u8] {
    if let Some(i) = raw.windows(2).position(|s| s == b"\0\x01") {
        &raw[..i]
    } else if let Some(i) = raw.windows(2).position(|s| s == b"::") {
        &raw[i + 2..]
    } else {
        raw
    }
}
pub(crate) fn key(doc: &Document<'_>, n: usize, a: &Budget) -> Result<String, Error> {
    fn hex(v: &[u8], a: &Budget) -> Result<String, Error> {
        let mut s = Vec::new();
        a.reserve(
            &mut s,
            v.len()
                .checked_mul(2)
                .ok_or(Error::LimitExceeded(crate::Limit::Allocation))?,
        )?;
        for b in v {
            s.push(b"0123456789abcdef"[usize::from(b >> 4)]);
            s.push(b"0123456789abcdef"[usize::from(b & 15)]);
        }
        a.owned_text(s)
    }
    a.format(format_args!(
        "legacy:{}:{}:ordinal:{:020}",
        hex(doc.nodes[n].name, a)?,
        hex(string(values(doc, n).first()), a)?,
        n.checked_add(1)
            .ok_or(Error::LimitExceeded(crate::Limit::Allocation))?
    ))
}
pub(crate) fn order(a: &str, b: &str) -> std::cmp::Ordering {
    if a == b {
        return std::cmp::Ordering::Equal;
    }
    if a.starts_with("legacy:") && b.starts_with("legacy:") {
        let mut a = a.split(':').skip(1);
        let mut b = b.split(':').skip(1);
        a.next()
            .cmp(&b.next())
            .then_with(|| a.next().cmp(&b.next()))
            .then_with(|| a.last().cmp(&b.last()))
    } else {
        a.cmp(b)
    }
}
fn push(
    elements: &mut Vec<Element>,
    node: usize,
    ty: T,
    name: &[u8],
    key: String,
    a: &Budget,
    max: usize,
) -> Result<usize, Error> {
    if elements.len() >= max {
        return Err(Error::LimitExceeded(crate::Limit::Objects));
    }
    let i = elements.len();
    a.push(
        elements,
        Element {
            key,
            ordinal: node
                .checked_add(1)
                .ok_or(Error::LimitExceeded(crate::Limit::Allocation))?,
            source_id: None,
            ty,
            node: Some(node),
            name: a.bytes(name)?,
            src_conns: Vec::new(),
            dst_conns: Vec::new(),
            instances: Vec::new(),
            obj: Obj::new(),
        },
    )?;
    Ok(i)
}
/// Create model attachments and flatten Takes while retaining original DOM nodes.
pub(crate) fn elements(
    doc: &Document<'_>,
    props: &PropertyIndex,
    elements: &mut Vec<Element>,
    conns: &mut Vec<(usize, usize, Vec<u8>)>,
    limits: &DumpLimits,
) -> Result<(), Error> {
    let a = crate::conversion_budget::Budget::new(
        limits.max_allocation,
        limits.max_animation_work,
        crate::conversion_budget::Path::Layers,
    );
    a.work(doc.nodes.len())?;
    for p in &doc.properties {
        if let Property::String(v) = p {
            a.work(v.len())?;
        }
    }
    let originals = elements.len();
    let mut attrs = a.filled(originals, None)?;
    for owner in 1..originals {
        if elements[owner].ty != T::Node {
            continue;
        }
        let Some(n) = elements[owner].node else {
            continue;
        };
        let sub = crate::scene::fbx_subtype(string(values(doc, n).get(1)));
        let ty = match sub {
            b"Mesh" => T::Mesh,
            b"Camera" => T::Camera,
            b"Light" => T::Light,
            b"LimbNode" | b"Limb" | b"Root" => T::Bone,
            b"Null" | b"Marker" => T::Empty,
            b"" | b"Model" if props.children(n).any(|c| doc.nodes[c].name == b"Vertices") => {
                T::Mesh
            }
            b"" | b"Model" => continue,
            _ => T::Unknown,
        };
        let raw = props
            .children(n)
            .find(|&c| doc.nodes[c].name == b"NodeAttributeName")
            .map(|c| string(values(doc, c).first()));
        let nm = raw
            .map(name)
            .filter(|v| !v.is_empty())
            .unwrap_or(&elements[owner].name);
        let nm = a.bytes(nm)?;
        let k = a.format(format_args!(
            "synthetic:{}:role:attachment:type:{}:ordinal:0",
            elements[owner].key, ty as u8
        ))?;
        let at = push(elements, n, ty, &nm, k, &a, limits.index.max_objects)?;
        attrs[owner] = Some(at);
        connect(
            conns,
            (at, owner, Vec::new()),
            &a,
            limits.index.max_connections,
        )?;
    }
    // Attribute properties were stored on Model in this format.
    for (src, dst, prop) in conns.iter_mut() {
        if (!prop.is_empty() && !node_property(prop))
            || (prop.is_empty() && matches!(elements[*src].ty, T::SkinDeformer | T::CacheDeformer))
        {
            if let Some(at) = attrs.get(*dst).copied().flatten() {
                *dst = at;
            }
        }
    }
    let mut names = a.collect(
        elements[..originals]
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.node.map(|n| (string(values(doc, n).first()), i))),
    )?;
    a.sort_work(names.len())?;
    names.sort_unstable();
    for (take, node) in doc.nodes.iter().enumerate() {
        if node.name != b"Take" || !node.parent.is_some_and(|p| doc.nodes[p].name == b"Takes") {
            continue;
        }
        let nm = string(values(doc, take).first());
        let stack = push(
            elements,
            take,
            T::AnimStack,
            nm,
            key(doc, take, &a)?,
            &a,
            limits.index.max_objects,
        )?;
        let k = a.format(format_args!(
            "synthetic:{}:role:take-layer:type:30:ordinal:0",
            elements[stack].key
        ))?;
        let layer = push(
            elements,
            take,
            T::AnimLayer,
            b"BaseLayer",
            k,
            &a,
            limits.index.max_objects,
        )?;
        connect(
            conns,
            (layer, stack, Vec::new()),
            &a,
            limits.index.max_connections,
        )?;
        for model in props
            .children(take)
            .filter(|&c| doc.nodes[c].name == b"Model")
        {
            let raw = string(values(doc, model).first());
            let Ok(k) = names.binary_search_by(|p| p.0.cmp(raw)) else {
                continue;
            };
            let target = names[k].1;
            for channel in props
                .children(model)
                .filter(|&c| doc.nodes[c].name == b"Channel")
            {
                let nm = string(values(doc, channel).first());
                if nm == b"Transform" {
                    for c in props
                        .children(channel)
                        .filter(|&c| doc.nodes[c].name == b"Channel")
                    {
                        let nm: &[u8] = match string(values(doc, c).first()) {
                            b"T" => b"Lcl Translation",
                            b"R" => b"Lcl Rotation",
                            b"S" => b"Lcl Scaling",
                            _ => continue,
                        };
                        channel_elements(
                            doc,
                            props,
                            elements,
                            conns,
                            Channel {
                                node: c,
                                target,
                                layer,
                                name: nm,
                            },
                            &a,
                            limits,
                        )?;
                    }
                } else {
                    let target = if node_property(nm) {
                        target
                    } else {
                        attrs[target].unwrap_or(target)
                    };
                    channel_elements(
                        doc,
                        props,
                        elements,
                        conns,
                        Channel {
                            node: channel,
                            target,
                            layer,
                            name: nm,
                        },
                        &a,
                        limits,
                    )?;
                }
            }
        }
    }
    if elements.len() > limits.index.max_objects {
        return Err(Error::LimitExceeded(crate::Limit::Objects));
    }
    if conns.len() > limits.index.max_connections {
        return Err(Error::LimitExceeded(crate::Limit::Connections));
    }
    Ok(())
}
struct Channel<'a> {
    node: usize,
    target: usize,
    layer: usize,
    name: &'a [u8],
}
fn channel_elements(
    doc: &Document<'_>,
    props: &PropertyIndex,
    es: &mut Vec<Element>,
    cs: &mut Vec<(usize, usize, Vec<u8>)>,
    channel: Channel<'_>,
    a: &crate::conversion_budget::Budget,
    limits: &DumpLimits,
) -> Result<(), Error> {
    let Channel {
        node: n,
        target,
        layer,
        name: nm,
    } = channel;
    let valid = |c| {
        props
            .children(c)
            .any(|k| matches!(doc.nodes[k].name, b"Key" | b"Default"))
    };
    let mut channels = Vec::new();
    if valid(n) {
        a.push(&mut channels, n)?;
    } else {
        for c in props
            .children(n)
            .filter(|&c| doc.nodes[c].name == b"Channel" && valid(c))
            .take(3)
        {
            a.push(&mut channels, c)?;
        }
    }
    a.work(channels.len() + 1)?;
    if channels.is_empty() {
        return Ok(());
    }
    let v = push(
        es,
        n,
        T::AnimValue,
        nm,
        key(doc, n, a)?,
        a,
        limits.index.max_objects,
    )?;
    connect(cs, (v, layer, Vec::new()), a, limits.index.max_connections)?;
    connect(
        cs,
        (v, target, a.bytes(nm)?),
        a,
        limits.index.max_connections,
    )?;
    for c in channels {
        if !props.children(c).any(|k| doc.nodes[k].name == b"Key") {
            continue;
        }
        let cn = if c == n {
            nm
        } else {
            string(values(doc, c).first())
        };
        let k = if c == n {
            a.format(format_args!(
                "synthetic:{}:role:component-curve:type:32:ordinal:0",
                es[v].key
            ))?
        } else {
            key(doc, c, a)?
        };
        let curve = push(es, c, T::AnimCurve, cn, k, a, limits.index.max_objects)?;
        connect(
            cs,
            (curve, v, a.bytes(cn)?),
            a,
            limits.index.max_connections,
        )?;
    }
    Ok(())
}

/// `ufbxi_is_node_property_name`, including user properties at the call site.
pub(crate) fn node_property(name: &[u8]) -> bool {
    matches!(
        name,
        b"AxisLen"
            | b"DefaultAttributeIndex"
            | b"Freeze"
            | b"GeometricRotation"
            | b"GeometricScaling"
            | b"GeometricTranslation"
            | b"InheritType"
            | b"LODBox"
            | b"Lcl Rotation"
            | b"Lcl Scaling"
            | b"Lcl Translation"
            | b"LookAtProperty"
            | b"MaxDampRangeX"
            | b"MaxDampRangeY"
            | b"MaxDampRangeZ"
            | b"MaxDampStrengthX"
            | b"MaxDampStrengthY"
            | b"MaxDampStrengthZ"
            | b"MinDampRangeX"
            | b"MinDampRangeY"
            | b"MinDampRangeZ"
            | b"MinDampStrengthX"
            | b"MinDampStrengthY"
            | b"MinDampStrengthZ"
            | b"NegativePercentShapeSupport"
            | b"PostRotation"
            | b"PreRotation"
            | b"PreferedAngleX"
            | b"PreferedAngleY"
            | b"PreferedAngleZ"
            | b"QuaternionInterpolate"
            | b"RotationActive"
            | b"RotationMax"
            | b"RotationMaxX"
            | b"RotationMaxY"
            | b"RotationMaxZ"
            | b"RotationMin"
            | b"RotationMinX"
            | b"RotationMinY"
            | b"RotationMinZ"
            | b"RotationOffset"
            | b"RotationOrder"
            | b"RotationPivot"
            | b"RotationSpaceForLimitOnly"
            | b"RotationStiffnessX"
            | b"RotationStiffnessY"
            | b"RotationStiffnessZ"
            | b"ScalingActive"
            | b"ScalingMax"
            | b"ScalingMaxX"
            | b"ScalingMaxY"
            | b"ScalingMaxZ"
            | b"ScalingMin"
            | b"ScalingMinX"
            | b"ScalingMinY"
            | b"ScalingMinZ"
            | b"ScalingOffset"
            | b"ScalingPivot"
            | b"Show"
            | b"TranslationActive"
            | b"TranslationMax"
            | b"TranslationMaxX"
            | b"TranslationMaxY"
            | b"TranslationMaxZ"
            | b"TranslationMin"
            | b"TranslationMinX"
            | b"TranslationMinY"
            | b"TranslationMinZ"
            | b"UpVectorProperty"
            | b"Visibility Inheritance"
            | b"Visibility"
            | b"notes"
    )
}

/// Synthetic take bounds and channel defaults (`ufbxi_read_take`, `_prop_channel`).
pub(crate) fn layers<'a>(
    doc: &'a Document<'a>,
    index: &PropertyIndex,
    n: usize,
    max: usize,
    limit: usize,
) -> Result<PropertyLayers<'a>, crate::scene_property::Error> {
    use crate::scene_property::Error as E;
    let mut out = Vec::new();
    let mut used = 0;
    let mut add = |name: &'a [u8], kind: Type, i: i64, f: f64| -> Result<(), E> {
        if out.len() >= max {
            return Err(E::LimitExceeded);
        }
        crate::allocation::reserve(&mut out, 1, &mut used, limit).map_err(|e| match e {
            crate::allocation::Error::Limit => E::LimitExceeded,
            crate::allocation::Error::Failed => E::AllocationFailed,
        })?;
        out.push(AuthoredProperty {
            source_node: n,
            name,
            kind,
            flags: 0x1101000,
            value_int: i,
            value_real: [f, 0.0, 0.0, 0.0],
            value_str: b"",
            value_blob: b"",
        });
        Ok(())
    };
    if doc.nodes[n].name == b"Take" {
        for (field, names) in [
            (
                b"LocalTime".as_slice(),
                [b"LocalStart".as_slice(), b"LocalStop"],
            ),
            (
                b"ReferenceTime",
                [b"ReferenceStart".as_slice(), b"ReferenceStop"],
            ),
        ] {
            if let Some(c) = index.children(n).find(|&c| doc.nodes[c].name == field) {
                for (p, nm) in values(doc, c).iter().zip(names) {
                    let i = match p {
                        Property::I64(v) => *v,
                        Property::I32(v) => i64::from(*v),
                        _ => return Err(E::UnsupportedValue),
                    };
                    add(nm, Type::Integer, i, i as f64)?;
                }
            }
        }
    }
    out.sort_unstable_by(|a, b| a.name.cmp(b.name));
    Ok(PropertyLayers {
        authored: out,
        defaults: None,
    })
}

pub(crate) fn defaults(
    doc: &Document<'_>,
    props: &PropertyIndex,
    n: usize,
    ascii: bool,
    a: &Budget,
) -> Result<[f64; 3], Error> {
    use crate::canon_dom::{node_data, NodeData, Scalar};
    let scalar = |n| -> Result<Option<f64>, Error> {
        props
            .children(n)
            .find(|&c| doc.nodes[c].name == b"Default")
            .map(|c| {
                Ok(match node_data(doc, c, ascii, a)? {
                    NodeData::Values(v) => v.first().and_then(|v| match v {
                        Scalar::Num(_, f) => Some(*f),
                        _ => None,
                    }),
                    _ => None,
                })
            })
            .transpose()
            .map(Option::flatten)
    };
    let mut v = [0.0; 3];
    if let Some(f) = scalar(n)? {
        v[0] = f;
    } else {
        for (i, c) in props
            .children(n)
            .filter(|&c| {
                doc.nodes[c].name == b"Channel"
                    && props
                        .children(c)
                        .any(|k| matches!(doc.nodes[k].name, b"Key" | b"Default"))
            })
            .take(3)
            .enumerate()
        {
            v[i] = scalar(c)?.unwrap_or(0.0);
        }
    }
    Ok(v)
}

/// `ufbxi_read_legacy_settings`: Version5/Settings (or root Settings) frame rate.
pub(crate) fn settings<'a>(
    doc: &'a Document<'a>,
    index: &PropertyIndex,
    out: &mut Vec<AuthoredProperty<'a>>,
    a: &Budget,
) -> Result<(), Error> {
    let settings = doc.nodes.iter().position(|n| {
        n.name == b"Settings"
            && (n.parent.is_none() || n.parent.is_some_and(|p| doc.nodes[p].name == b"Version5"))
    });
    let Some(settings) = settings else {
        return Ok(());
    };
    let Some(frame) = index
        .children(settings)
        .find(|&n| doc.nodes[n].name == b"FrameRate")
    else {
        return Ok(());
    };
    let fps = match values(doc, frame).first() {
        Some(Property::F64(v)) => f64::from_bits(*v),
        Some(Property::F32(v)) => f64::from(f32::from_bits(*v)),
        Some(Property::I64(v)) => *v as f64,
        Some(Property::I32(v)) => f64::from(*v),
        Some(Property::String(v)) => std::str::from_utf8(v)
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0),
        _ => 0.0,
    };
    if fps > 0.0 {
        for (nm, ty, v) in [
            (b"CustomFrameRate".as_slice(), Type::Number, fps),
            (b"TimeMode", Type::Integer, 14.0),
        ] {
            if out.iter().any(|p| p.name == nm) {
                continue;
            }
            a.push(
                out,
                AuthoredProperty {
                    source_node: frame,
                    name: nm,
                    kind: ty,
                    flags: 0x101000,
                    value_int: crate::numeric::saturating_i64(v),
                    value_real: [v, 0.0, 0.0, 0.0],
                    value_str: b"",
                    value_blob: b"",
                },
            )?;
        }
        out.sort_unstable_by(|a, b| a.name.cmp(b.name));
    }
    Ok(())
}

fn connect(
    out: &mut Vec<(usize, usize, Vec<u8>)>,
    c: (usize, usize, Vec<u8>),
    a: &Budget,
    max: usize,
) -> Result<(), Error> {
    if out.len() >= max {
        return Err(Error::LimitExceeded(crate::Limit::Connections));
    }
    a.push(out, c)
}

/// `ufbxi_read_mesh` and the LayerElement texture patch in `ufbxi_finalize_scene`.
/// Patch the material projection inputs, not the authored connection records.
pub(crate) fn material_textures(
    scene: &crate::canon_scene::CanonScene<'_>,
    items: &mut [crate::scene_material::Item<'_>],
    limit: usize,
) -> Result<(), Error> {
    use crate::canon::V;
    use crate::canon_dom::{node_data, NodeData};
    let a = Budget::new(limit);
    let mut work = 0usize;
    fn charge(work: &mut usize, n: usize) -> Result<(), Error> {
        *work = work
            .checked_add(n)
            .ok_or(Error::ResourceLimit("legacy texture work"))?;
        if *work > 1 << 26 {
            return Err(Error::ResourceLimit("legacy texture work"));
        }
        Ok(())
    }
    let mut item_index = a.filled(scene.elements.len(), None)?;
    for (i, it) in items.iter().enumerate() {
        item_index[it.element] = Some(i);
    }
    let mut typed_mesh = a.filled(scene.elements.len(), None)?;
    for (i, m, _) in &scene.typed.meshes {
        typed_mesh[*i] = Some(m);
    }
    let mut meshes = a.collect(
        scene
            .elements
            .iter()
            .enumerate()
            .filter_map(|(i, e)| (e.ty == T::Mesh).then_some(i)),
    )?;
    meshes.sort_unstable_by_key(|&i| scene.created(i));
    for mesh in meshes {
        let e = &scene.elements[mesh];
        let Some(node) = e.node else {
            continue;
        };
        let groups = [Some(mesh), e.instances.first().copied()];
        let fetch = |owner: usize, ty: T| -> Result<Vec<usize>, Error> {
            let mut list = a.collect(
                scene.elements[owner]
                    .dst_conns
                    .iter()
                    .filter(|(s, p)| p.is_empty() && scene.elements[*s].ty == ty)
                    .map(|(s, _)| *s),
            )?;
            list.sort_unstable_by_key(|&i| scene.created(i));
            Ok(list)
        };
        let mut materials = Vec::new();
        let mut textures = Vec::new();
        for owner in groups.into_iter().flatten() {
            charge(&mut work, scene.elements[owner].dst_conns.len())?;
            if materials.is_empty() {
                materials = fetch(owner, T::Material)?;
            }
            let next = fetch(owner, T::Texture)?;
            a.reserve(&mut textures, next.len())?;
            textures.extend(next);
        }
        if materials.is_empty() || textures.is_empty() {
            continue;
        }
        let face_material: &[u32] = if let Some(m) = typed_mesh[mesh] {
            m.face_materials()
        } else {
            match e.obj.get("face_material") {
                Some(V::U32s(v)) => v,
                _ => &[],
            }
        };
        let mut rows: Vec<(usize, &[u8], usize)> = Vec::new();
        for layer in scene.props.children(node) {
            charge(&mut work, 1)?;
            let name = scene.doc.nodes[layer].name;
            let prop = if name == b"LayerElementTexture" {
                b"Diffuse".as_slice()
            } else if name.starts_with(b"LayerElement")
                && name.len() > 20
                && name.ends_with(b"Textures")
            {
                let prop = &name[12..name.len() - 8];
                prop.strip_suffix(b"_").unwrap_or(prop)
            } else {
                continue;
            };
            if name.contains(&0) {
                return Err(Error::InvalidData("legacy texture layer name"));
            }
            let Some(mapping) = scene
                .props
                .children(layer)
                .find(|&c| scene.doc.nodes[c].name == b"MappingInformationType")
            else {
                continue;
            };
            let all_same = string(values(&scene.doc, mapping).first()) == b"AllSame";
            let ids = if let Some(c) = scene
                .props
                .children(layer)
                .find(|&c| scene.doc.nodes[c].name == b"TextureId")
            {
                charge(&mut work, crate::canon_dom::data_len(&scene.doc, c)?)?;
                match node_data(&scene.doc, c, scene.ascii, &a)? {
                    NodeData::Ints(v) => v,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            if all_same {
                let tex = ids.first().copied().unwrap_or(0);
                if let Ok(tex) = usize::try_from(tex) {
                    if tex < textures.len() {
                        charge(&mut work, materials.len())?;
                        for slot in 0..materials.len() {
                            a.push(&mut rows, (slot, prop, tex))?;
                        }
                    }
                }
            } else {
                let mut prev = None;
                for (&tex, &mat) in ids.iter().zip(face_material) {
                    charge(&mut work, 1)?;
                    let (Ok(tex), Ok(mat)) = (usize::try_from(tex), usize::try_from(mat)) else {
                        continue;
                    };
                    if tex >= textures.len() || mat >= materials.len() || prev == Some((mat, tex)) {
                        continue;
                    }
                    prev = Some((mat, tex));
                    a.push(&mut rows, (mat, prop, tex))?;
                }
            }
        }
        let sort_cost = rows
            .len()
            .checked_mul(
                usize::try_from(usize::BITS - rows.len().leading_zeros())
                    .map_err(|_| Error::ResourceLimit("legacy texture work"))?
                    .checked_add(1)
                    .ok_or(Error::ResourceLimit("legacy texture work"))?,
            )
            .ok_or(Error::ResourceLimit("legacy texture work"))?;
        charge(&mut work, sort_cost)?;
        rows.sort_unstable();
        rows.dedup();
        let mut start = 0;
        while start < rows.len() {
            let slot = rows[start].0;
            let end = start + rows[start..].partition_point(|r| r.0 == slot);
            if let Some(i) = item_index[materials[slot]] {
                if !items[i]
                    .dst
                    .iter()
                    .any(|(s, _)| scene.elements[*s].ty == T::Texture)
                {
                    for &(_, prop, tex) in &rows[start..end] {
                        a.push(&mut items[i].dst, (textures[tex], a.bytes(prop)?))?;
                    }
                }
            }
            start = end;
        }
    }
    Ok(())
}
