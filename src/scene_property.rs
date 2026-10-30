//! Bounded projection of authored FBX property lists, for future canonical
//! output. Defaults, connection/animation flags and synthetic properties are
//! deliberately not synthesized here.
//!
//! Based on ufbx 0.23.1 `ufbxi_read_property`, `ufbxi_read_properties` and
//! `ufbxi_deduplicate_properties` (MIT):
//! <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>
use crate::syntax::{Document, Property};
use std::mem::size_of;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidDom,
    UnsupportedString,
    UnsupportedBlob,
    UnsupportedValue,
    LimitExceeded,
    AllocationFailed,
}

/// Public `ufbx_prop_type` discriminants in the pinned header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Type {
    Unknown,
    Boolean,
    Integer,
    Number,
    Vector,
    Color,
    ColorWithAlpha,
    String,
    DateTime,
    Translation,
    Rotation,
    Scaling,
    Distance,
    Compound,
    Blob,
    Reference,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredProperty<'a> {
    pub source_node: usize,
    pub name: &'a [u8],
    pub kind: Type,
    pub flags: u32,
    pub value_int: i64,
    pub value_real: [f64; 4],
    pub value_str: &'a [u8],
    pub value_blob: &'a [u8],
}

/// Separate authored and source-template property lists. This does not include
/// synthesized defaults, connection flags, or animation modifications.
#[derive(Debug)]
pub struct PropertyLayers<'a> {
    pub authored: Vec<AuthoredProperty<'a>>,
    pub defaults: Option<Vec<AuthoredProperty<'a>>>,
}

impl<'a> PropertyLayers<'a> {
    pub fn find(&self, name: &[u8]) -> Option<&AuthoredProperty<'a>> {
        fn lookup<'a, 'b>(
            list: &'b [AuthoredProperty<'a>],
            name: &[u8],
        ) -> Option<&'b AuthoredProperty<'a>> {
            list.binary_search_by(|p| p.name.cmp(name))
                .ok()
                .map(|i| &list[i])
        }
        lookup(&self.authored, name)
            .or_else(|| self.defaults.as_deref().and_then(|v| lookup(v, name)))
    }
}

/// Build both retained lists within a shared vector-capacity budget.
pub fn layers<'a>(
    doc: &'a Document<'a>,
    owner: usize,
    max_properties: usize,
    max_allocation: usize,
) -> Result<PropertyLayers<'a>, Error> {
    let node = doc.nodes.get(owner).ok_or(Error::InvalidDom)?;
    let p = doc
        .properties
        .get(node.properties.clone())
        .ok_or(Error::InvalidDom)?;
    let subtype = p.get(2).and_then(string).ok_or(Error::InvalidDom)?;
    let template = template_owner(doc, node.name, subtype)?;
    let direct = authored(doc, owner, max_properties, max_allocation)?;
    let used = direct
        .capacity()
        .checked_mul(size_of::<AuthoredProperty<'_>>())
        .ok_or(Error::LimitExceeded)?;
    let remaining = max_allocation
        .checked_sub(used)
        .ok_or(Error::LimitExceeded)?;
    let count_left = max_properties
        .checked_sub(direct.capacity())
        .ok_or(Error::LimitExceeded)?;
    let defaults = match template {
        Some(t) => {
            let value = authored(doc, t, count_left, remaining)?;
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        }
        None => None,
    };
    Ok(PropertyLayers {
        authored: direct,
        defaults,
    })
}

/// Locate the source template owner selected for an object. This only selects
/// the source list; callers must retain it as a separate defaults chain rather
/// than merging it into the authored list. As in ufbx, the first ObjectType
/// with the given name decides the result even if its subtype does not match.
pub fn template_owner(
    doc: &Document<'_>,
    object_type: &[u8],
    subtype: &[u8],
) -> Result<Option<usize>, Error> {
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
        let name = simple_string(p.first().and_then(string).ok_or(Error::InvalidDom)?)?;
        if name != object_type {
            continue;
        }
        let template = doc
            .nodes
            .iter()
            .enumerate()
            .find(|(_, t)| t.parent == Some(i) && t.name == b"PropertyTemplate");
        let Some((ti, t)) = template else {
            return Ok(None);
        };
        let p = doc
            .properties
            .get(t.properties.clone())
            .ok_or(Error::InvalidDom)?;
        let template_subtype = simple_string(p.first().and_then(string).ok_or(Error::InvalidDom)?)?;
        let template_subtype = crate::scene::fbx_subtype(template_subtype);
        let template_subtype = if template_subtype == b"LODGroup" {
            &b"LodGroup"[..]
        } else {
            template_subtype
        };
        if !matches!(
            object_type,
            b"Material" | b"Model" | b"AnimationStack" | b"AnimationLayer"
        ) && template_subtype != crate::scene::fbx_subtype(subtype)
        {
            return Ok(None);
        }
        let property_list = doc
            .nodes
            .iter()
            .enumerate()
            .find(|(_, n)| n.parent == Some(ti) && n.name == b"Properties70")
            .or_else(|| {
                doc.nodes
                    .iter()
                    .enumerate()
                    .find(|(_, n)| n.parent == Some(ti) && n.name == b"Properties60")
            });
        if !property_list.is_some_and(|(li, _)| doc.nodes.iter().any(|n| n.parent == Some(li))) {
            return Ok(None);
        }
        return Ok(Some(ti));
    }
    Ok(None)
}

fn string<'a>(p: &'a Property<'_>) -> Option<&'a [u8]> {
    match p {
        Property::String(v) => Some(v),
        _ => None,
    }
}
fn simple_string(bytes: &[u8]) -> Result<&[u8], Error> {
    // String sanitization and raw-context selection need the ufbx parse
    // context; never claim this restricted view covers those cases.
    if bytes.iter().all(|&b| b > 0 && b < 128) {
        Ok(bytes)
    } else {
        Err(Error::UnsupportedString)
    }
}
fn property_type(s: &[u8]) -> Type {
    match s {
        b"Boolean" | b"bool" | b"Bool" => Type::Boolean,
        b"Integer"
        | b"int"
        | b"enum"
        | b"Enum"
        | b"Visibility"
        | b"Visibility Inheritance"
        | b"KTime" => Type::Integer,
        b"Number" | b"double" | b"Real" | b"Float" | b"Intensity" => Type::Number,
        b"Vector" | b"Vector3D" => Type::Vector,
        b"Color" | b"ColorRGB" => Type::Color,
        b"ColorAndAlpha" => Type::ColorWithAlpha,
        b"String" | b"KString" | b"object" => Type::String,
        b"DateTime" => Type::DateTime,
        b"Lcl Translation" => Type::Translation,
        b"Lcl Rotation" => Type::Rotation,
        b"Lcl Scaling" => Type::Scaling,
        b"Distance" => Type::Distance,
        b"Compound" => Type::Compound,
        b"Blob" => Type::Blob,
        b"Reference" => Type::Reference,
        _ => Type::Unknown,
    }
}
fn number(p: &Property<'_>) -> Option<(i64, f64)> {
    let (integer, real) = match p {
        Property::I16(v) => (i64::from(*v), f64::from(*v)),
        Property::I32(v) => (i64::from(*v), f64::from(*v)),
        Property::I64(v) => (*v, *v as f64),
        Property::Bool(v) | Property::BoolAlt(v) => (
            i64::from(i8::from_ne_bytes([*v])),
            f64::from(i8::from_ne_bytes([*v])),
        ),
        Property::AsciiBare(v) => crate::syntax::bare_number(v),
        Property::AsciiNegativeZero => (0, -0.0),
        Property::F32(bits) => {
            let v = f64::from(f32::from_bits(*bits));
            (float_int(v), v)
        }
        Property::F64(bits) => {
            let v = f64::from_bits(*bits);
            (float_int(v), v)
        }
        _ => return None,
    };
    Some((integer, real))
}
fn float_int(v: f64) -> i64 {
    // ufbx uses the x86 conversion result for NaN and the +2^63 boundary.
    if v.is_nan() || v == 9_223_372_036_854_775_808.0 {
        i64::MIN
    } else {
        crate::numeric::saturating_i64(v)
    }
}

const MAX_WORK: usize = 1 << 26;
#[derive(Default)]
struct Work {
    actual: usize,
    ranges: usize,
}
impl Work {
    fn add(&mut self, n: usize) -> Result<(), Error> {
        self.actual = self.actual.checked_add(n).ok_or(Error::LimitExceeded)?;
        if self.actual > MAX_WORK {
            return Err(Error::LimitExceeded);
        }
        #[cfg(test)]
        PROPERTY_WORK.with(|v| v.set(v.get() + n));
        Ok(())
    }
    fn range(&mut self, n: usize) -> Result<(), Error> {
        self.ranges = self.ranges.checked_add(n).ok_or(Error::LimitExceeded)?;
        if self.ranges > MAX_WORK {
            return Err(Error::LimitExceeded);
        }
        self.add(1)
    }
    fn string<'a>(&mut self, s: &'a [u8]) -> Result<&'a [u8], Error> {
        self.add(s.len())?;
        simple_string(s)
    }
}
#[cfg(test)]
thread_local! { static PROPERTY_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Read the first Properties70 (or Properties60) child of `owner`. The list
/// sorts by name and retains the last occurrence of each duplicate. This
/// restricted function rejects non-ASCII strings and nested binary values.
pub fn authored<'a>(
    doc: &'a Document<'a>,
    owner: usize,
    max_properties: usize,
    max_allocation: usize,
) -> Result<Vec<AuthoredProperty<'a>>, Error> {
    if owner >= doc.nodes.len() {
        return Err(Error::InvalidDom);
    }
    let find = |name: &[u8]| {
        doc.nodes
            .iter()
            .enumerate()
            .find(|(_, n)| n.parent == Some(owner) && n.name == name)
            .map(|(i, _)| i)
    };
    let (container, value_offset) = if let Some(i) = find(b"Properties70") {
        (i, 4)
    } else if let Some(i) = find(b"Properties60") {
        (i, 3)
    } else {
        return Ok(Vec::new());
    };
    let count = doc
        .nodes
        .iter()
        .filter(|n| n.parent == Some(container))
        .count();
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut work = Work::default();
    work.add(doc.nodes.len().checked_mul(5).ok_or(Error::LimitExceeded)?)?;
    if count > max_properties
        || count
            .checked_mul(size_of::<AuthoredProperty<'_>>())
            .is_none_or(|v| v > max_allocation)
    {
        return Err(Error::LimitExceeded);
    }
    // Locate the earliest property with a direct BinaryData child once. Using
    // its index preserves validation order (and errors before it), without a
    // document scan per property or any additional allocation. This also works
    // for caller-constructed DOMs whose sibling subtrees are interleaved.
    let blob_property = doc
        .nodes
        .iter()
        .filter(|n| n.name == b"BinaryData")
        .filter_map(|n| n.parent)
        .filter(|&p| {
            doc.nodes
                .get(p)
                .is_some_and(|n| n.parent == Some(container))
        })
        .min();
    let mut used = 0;
    let allocation_error = |e| match e {
        crate::allocation::Error::Limit => Error::LimitExceeded,
        crate::allocation::Error::Failed => Error::AllocationFailed,
    };
    let mut out = Vec::new();
    crate::allocation::reserve(&mut out, count, &mut used, max_allocation)
        .map_err(allocation_error)?;
    // Prefix counts index every stored value once, including overlapping and
    // shared caller-constructed ranges. Queries retain per-node error order.
    work.add(doc.properties.len())?;
    let mut unsupported = Vec::<usize>::new();
    let prefix_len = doc
        .properties
        .len()
        .checked_add(1)
        .ok_or(Error::LimitExceeded)?;
    crate::allocation::reserve(&mut unsupported, prefix_len, &mut used, max_allocation)
        .map_err(allocation_error)?;
    unsupported.push(0);
    for p in &doc.properties {
        let bad = matches!(
            p,
            Property::AsciiArray(_) | Property::Array { .. } | Property::Raw(_)
        );
        unsupported.push(unsupported.last().unwrap() + usize::from(bad));
    }
    for (i, node) in doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent == Some(container))
    {
        let p = doc
            .properties
            .get(node.properties.clone())
            .ok_or(Error::InvalidDom)?;
        work.range(p.len())?;
        if unsupported[node.properties.end] != unsupported[node.properties.start] {
            return Err(Error::UnsupportedValue);
        }
        let name = work.string(p.first().and_then(string).ok_or(Error::InvalidDom)?)?;
        let type_name = work.string(p.get(1).and_then(string).ok_or(Error::InvalidDom)?)?;
        let mut kind = property_type(type_name);
        if kind == Type::Unknown && value_offset == 4 {
            if let Some(subtype) = p.get(2).and_then(string) {
                kind = property_type(work.string(subtype)?);
            }
        }
        let mut flags = 0u32;
        if let Some(s) = p.get(value_offset - 1).and_then(string) {
            work.add(s.len())?;
            for (pos, c) in s.iter().copied().enumerate() {
                let next = s.get(pos + 1).copied().unwrap_or(b'0');
                match c {
                    b'A' => flags |= 1,
                    b'U' => flags |= 2,
                    b'H' => flags |= 4,
                    b'L' => flags |= u32::from(next.wrapping_sub(b'0') & 15) << 4,
                    b'M' => flags |= u32::from(next.wrapping_sub(b'0') & 15) << 8,
                    _ => (),
                }
            }
        }
        let mut value_int = 0;
        if let Some((v, _)) = p.get(value_offset).and_then(number) {
            value_int = v;
            flags |= 0x1000000;
        }
        let mut value_real = [0.0; 4];
        for (j, value) in value_real.iter_mut().enumerate() {
            let Some((_, n)) = p.get(value_offset + j).and_then(number) else {
                break;
            };
            *value = n;
            flags |= 0x100000 << j;
        }
        // The real-value flags are a single component-count bit, not a mask.
        if flags & 0xf00000 != 0 {
            let bits = flags & 0xf00000;
            flags = (flags & !0xf00000) | (1 << (31 - bits.leading_zeros()));
        }
        let str_offset = value_offset + usize::from(p.get(value_offset).and_then(string).is_none());
        let mut value_str = &b""[..];
        let mut value_blob = &b""[..];
        if let Some(s) = p.get(str_offset).and_then(string) {
            value_str = work.string(s)?;
            value_blob = s;
            flags |= 0x2000000;
        }
        if blob_property == Some(i) {
            return Err(Error::UnsupportedBlob);
        }
        out.push(AuthoredProperty {
            source_node: i,
            name,
            kind,
            flags,
            value_int,
            value_real,
            value_str,
            value_blob,
        });
    }
    sort_properties(&mut out, &mut work)?;
    let mut retained = 0;
    for read in 0..out.len() {
        let duplicate = if retained == 0 {
            false
        } else {
            work.add(
                out[read]
                    .name
                    .len()
                    .min(out[retained - 1].name.len())
                    .saturating_add(1),
            )?;
            out[read].name == out[retained - 1].name
        };
        if duplicate {
            out[retained - 1] = out[read].clone();
        } else {
            out.swap(retained, read);
            retained += 1;
        }
    }
    out.truncate(retained);
    Ok(out)
}

fn sort_properties(out: &mut [AuthoredProperty<'_>], work: &mut Work) -> Result<(), Error> {
    fn less(
        a: &AuthoredProperty<'_>,
        b: &AuthoredProperty<'_>,
        work: &mut Work,
    ) -> Result<bool, Error> {
        work.add(a.name.len().min(b.name.len()).saturating_add(1))?;
        Ok(a.name
            .cmp(b.name)
            .then(a.source_node.cmp(&b.source_node))
            .is_lt())
    }
    fn sift(
        out: &mut [AuthoredProperty<'_>],
        mut root: usize,
        end: usize,
        work: &mut Work,
    ) -> Result<(), Error> {
        while root < end / 2 {
            let mut child = root * 2 + 1;
            if child + 1 < end && less(&out[child], &out[child + 1], work)? {
                child += 1;
            }
            if !less(&out[root], &out[child], work)? {
                break;
            }
            out.swap(root, child);
            root = child;
        }
        Ok(())
    }
    for root in (0..out.len() / 2).rev() {
        sift(out, root, out.len(), work)?;
    }
    for end in (1..out.len()).rev() {
        out.swap(0, end);
        sift(out, 0, end, work)?;
    }
    Ok(())
}

#[cfg(test)]
mod hardening_tests {
    use super::*;
    use crate::syntax::Node;
    use std::borrow::Cow;
    fn shared(n: usize, name: &[u8]) -> Document<'_> {
        let mut properties = vec![Property::I64(7); n.max(5)];
        properties[0] = Property::String(Cow::Borrowed(name));
        properties[1] = Property::String(Cow::Borrowed(b"Number"));
        properties[2] = Property::String(Cow::Borrowed(b""));
        properties[3] = Property::String(Cow::Borrowed(b"A"));
        let mut nodes = vec![
            Node {
                name: b"Model",
                parent: None,
                properties: 0..0,
            },
            Node {
                name: b"Properties70",
                parent: Some(0),
                properties: 0..0,
            },
        ];
        for _ in 0..n {
            nodes.push(Node {
                name: b"P",
                parent: Some(1),
                properties: 0..properties.len(),
            });
        }
        Document {
            version: 7400,
            nodes,
            properties,
        }
    }
    #[test]
    fn shared_property_scaling() {
        let measure = |n| {
            let doc = shared(n, b"shared");
            PROPERTY_WORK.with(|c| c.set(0));
            let p = authored(&doc, 0, n, 16 << 20).unwrap();
            assert_eq!(p.len(), 1);
            assert_eq!(p[0].source_node, n + 1);
            assert_eq!(p[0].value_real, [7.0; 4]);
            PROPERTY_WORK.with(|c| c.get())
        };
        let small = measure(1000);
        let large = measure(8000);
        eprintln!(
            "properties: N=1000 {small}, 8N=8000 {large}, ratio={:.4}",
            large as f64 / small as f64
        );
        assert!(small > 0 && large > small && large < small * 14);
    }
    #[test]
    fn aggregate_ranges_and_shared_strings_are_bounded() {
        assert_eq!(
            authored(&shared(8193, b"shared"), 0, 8193, 16 << 20),
            Err(Error::LimitExceeded)
        );
        let name = vec![b'a'; 1 << 20];
        assert_eq!(
            authored(&shared(100, &name), 0, 100, 16 << 20),
            Err(Error::LimitExceeded)
        );
    }
    #[test]
    fn overlapping_ranges_retain_errors_and_last_duplicate() {
        let mut doc = shared(10, b"shared");
        // The bad value is outside the first range, inside all later ones.
        doc.properties[9] = Property::Raw(b"bad");
        doc.nodes[2].properties.end = 9;
        assert_eq!(authored(&doc, 0, 10, 4096), Err(Error::UnsupportedValue));
        doc.nodes[2].properties = 20..21;
        assert_eq!(authored(&doc, 0, 10, 4096), Err(Error::InvalidDom));
    }
    #[test]
    fn index_and_output_share_the_allocation_budget() {
        let doc = shared(5, b"shared");
        let output = 5 * size_of::<AuthoredProperty<'_>>();
        assert_eq!(authored(&doc, 0, 5, output), Err(Error::LimitExceeded));
        assert!(authored(&doc, 0, 5, output + 6 * size_of::<usize>()).is_ok());
    }
}
