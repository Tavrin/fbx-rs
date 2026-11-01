//! FBX 5.8/6.x and 7.1–7.7 syntax layer: binary and ASCII documents into a bounded flat DOM.
//! See docs/VERIFICATION.md for how it is checked against ufbx.
use std::borrow::Cow;

mod ascii;
use std::mem::size_of;

/// Resource limits for one [`parse`] call. Each count is checked before the
/// corresponding allocation.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum node nesting depth.
    pub max_depth: usize,
    /// Maximum number of nodes in the document.
    pub max_nodes: usize,
    /// Maximum number of property values in the document.
    pub max_properties: usize,
    /// Maximum number of array elements, summed over all arrays.
    pub max_array_elements: usize,
    /// Maximum bytes produced by decompressing arrays, in total.
    pub max_decompressed_bytes: usize,
    /// Maximum length of one string or raw property, in bytes.
    pub max_string_length: usize,
    /// Maximum bytes the parser may hold allocated at once.
    pub max_allocation: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_nodes: 1 << 24,
            max_properties: 1 << 25,
            max_array_elements: 1 << 27,
            max_decompressed_bytes: 1 << 30,
            max_string_length: 16 << 20,
            max_allocation: crate::allocation::DEFAULT_MAX_ALLOCATION,
        }
    }
}
/// Errors returned by [`parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Nodes are nested deeper than [`Limits::max_depth`].
    TooDeep,
    /// A [`Limits`] value was exceeded.
    LimitExceeded {
        /// Which limit.
        what: &'static str,
    },
    /// The input ends inside a header, node, property or array.
    Truncated,
    /// A compressed array is not valid zlib data or has the wrong length.
    BadCompression,
    /// The file version is outside the ranges accepted by [`supported_version`].
    UnsupportedVersion {
        /// The version number found in the file.
        v: u32,
    },
    /// An array uses an encoding other than raw (0) or zlib (1).
    UnsupportedEncoding {
        /// The encoding number found in the file.
        encoding: u32,
    },
    /// The input is not a well-formed FBX document.
    InvalidFormat,
    /// An ASCII token uses a syntax this parser does not accept.
    UnsupportedSyntax,
    /// The allocator refused a reservation within the limits.
    AllocationFailed,
}
/// One element of an ASCII array, kept as close to the source as possible.
#[derive(Debug, Clone, PartialEq)]
pub enum AsciiValue<'a> {
    /// An integer token.
    I64(i64),
    /// A floating-point token, as the bits of an `f64`.
    F64(u64),
    /// An unquoted word, as source bytes.
    Bare(&'a [u8]),
    /// An integer token written as `-0`.
    NegativeZero,
}
/// One property value of a node. Floating-point values are kept as their
/// bit patterns so that they round-trip exactly.
#[derive(Debug, Clone, PartialEq)]
pub enum Property<'a> {
    /// A 16-bit integer (binary type code `Y`).
    I16(i16),
    /// A boolean byte (binary type code `C`).
    Bool(u8),
    /// A boolean byte with the alternative type code `B`.
    BoolAlt(u8),
    /// A 32-bit integer (binary type code `I`).
    I32(i32),
    /// An `f32`, as its bits (binary type code `F`).
    F32(u32),
    /// An `f64`, as its bits (binary type code `D`).
    F64(u64),
    /// A 64-bit integer (binary type code `L`).
    I64(i64),
    /// A string (binary type code `S`), as bytes; in ASCII files the
    /// `&quot;`-style escapes are decoded.
    String(Cow<'a, [u8]>),
    /// Raw binary data (binary type code `R`).
    Raw(&'a [u8]),
    /// An unquoted ASCII word, preserved as source bytes.
    AsciiBare(&'a [u8]),
    /// An ASCII integer token with a negative zero sign.
    AsciiNegativeZero,
    /// ASCII array elements preserve numeric values and bare-word bytes.
    AsciiArray(Vec<AsciiValue<'a>>),
    /// Array bytes are in file byte order; `big_endian` indicates their interpretation.
    Array {
        /// The binary array type code: `f`, `d`, `l`, `i`, `b` or `c`.
        kind: u8,
        /// Number of elements.
        count: usize,
        /// Whether the elements are big-endian.
        big_endian: bool,
        /// The element bytes, decompressed when the file stores them
        /// compressed.
        bytes: Cow<'a, [u8]>,
    },
}
/// A node of the document tree.
#[derive(Debug)]
pub struct Node<'a> {
    /// The node name, as source bytes.
    pub name: &'a [u8],
    /// Index of the parent node in [`Document::nodes`]; `None` at top level.
    pub parent: Option<usize>,
    /// The node's values, as a range of [`Document::properties`].
    pub properties: std::ops::Range<usize>,
}
/// A parsed FBX document: nodes in preorder and their property values.
#[derive(Debug)]
pub struct Document<'a> {
    /// The FBX file version, for example 7400.
    pub version: u32,
    /// All nodes in preorder; a parent always precedes its children.
    pub nodes: Vec<Node<'a>>,
    /// The property values of all nodes, in node order.
    pub properties: Vec<Property<'a>>,
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    be: bool,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let bytes = self.bytes.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }
    fn num(&mut self, n: usize) -> Result<u64, Error> {
        let be = self.be;
        let data = self.take(n)?;
        let mut result = 0u64;
        if be {
            for &b in data {
                result = (result << 8) | u64::from(b);
            }
        } else {
            for (i, &b) in data.iter().enumerate() {
                result |= u64::from(b) << (i * 8);
            }
        }
        Ok(result)
    }
    fn usize(&mut self, n: usize) -> Result<usize, Error> {
        usize::try_from(self.num(n)?).map_err(|_| Error::LimitExceeded { what: "length" })
    }
}
// Bare words retain integer zero for special numbers (unlike signed numeric
// tokens, whose NaN integer conversion is i64::MIN), as in ufbx's token reader.
pub(crate) fn bare_number(word: &[u8]) -> (i64, f64) {
    if word.len() > 1 && word.len() < 64 {
        if let Some(n) = ascii::special_number(word) {
            return (0, n);
        }
    }
    let n = word
        .first()
        .map_or(0, |&b| i64::from(i8::from_ne_bytes([b])));
    (n, n as f64)
}
fn allocation_error(e: crate::allocation::Error) -> Error {
    match e {
        crate::allocation::Error::Limit => Error::LimitExceeded { what: "allocation" },
        crate::allocation::Error::Failed => Error::AllocationFailed,
    }
}
fn push<T>(vec: &mut Vec<T>, value: T, total: &mut usize, limit: usize) -> Result<(), Error> {
    if vec.len() == vec.capacity() {
        // A growth may temporarily keep both old and new allocations live.
        let minimum = vec
            .len()
            .checked_add(1)
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
        let preferred = vec
            .capacity()
            .checked_mul(2)
            .unwrap_or(minimum)
            .max(4)
            .max(minimum);
        let preferred_bytes = preferred.checked_mul(size_of::<T>());
        let fits = preferred_bytes
            .and_then(|n| total.checked_add(n))
            .is_some_and(|n| n <= limit);
        let target = if fits { preferred } else { minimum };
        crate::allocation::reserve(vec, target - vec.len(), total, limit)
            .map_err(allocation_error)?;
    }
    vec.push(value);
    Ok(())
}
/// Oldest accepted FBX version (5.8).
pub const MIN_VERSION: u32 = 5800;
/// Newest accepted FBX version (7.7).
pub const MAX_VERSION: u32 = 7700;

/// Whether `version` is in an accepted range: 5800..7000 or 7100..=7700.
pub fn supported_version(version: u32) -> bool {
    (MIN_VERSION..7000).contains(&version) || (7100..=MAX_VERSION).contains(&version)
}

/// Parse a binary or ASCII FBX document (5.8/6.x or 7.1–7.7).
pub fn parse<'a>(input: &'a [u8], limits: &Limits) -> Result<Document<'a>, Error> {
    const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a";
    if input.len() < MAGIC.len() && MAGIC.starts_with(input) {
        return Err(Error::Truncated);
    }
    if !input.starts_with(MAGIC) {
        return ascii::parse(input, limits);
    }
    parse_binary(input, limits)
}

// Validated binary curve subtrees can be restored one at a time by the lean
// loader. Source bytes remain borrowed from the caller, never copied.
#[derive(Debug, Clone, Copy)]
pub(crate) enum CompactConnectionKind {
    Object,
    Property,
    Invalid,
}
#[derive(Debug)]
pub(crate) struct CompactConnection<'a> {
    /// Original zero-based syntax ordinal, for diagnostics only.
    pub(crate) node: usize,
    pub(crate) source_id: i64,
    pub(crate) destination_id: i64,
    pub(crate) property: &'a [u8],
    pub(crate) kind: CompactConnectionKind,
}
impl<'a> CompactConnection<'a> {
    fn from_properties(node: usize, p: &[Property<'a>]) -> Self {
        fn text<'a>(p: &Property<'a>) -> Option<&'a [u8]> {
            match p {
                Property::String(Cow::Borrowed(b)) | Property::AsciiBare(b) => Some(b),
                _ => None,
            }
        }
        fn id(p: &Property<'_>) -> Option<i64> {
            match p {
                Property::I64(v) => Some(*v),
                Property::I32(v) => Some(i64::from(*v)),
                Property::I16(v) => Some(i64::from(*v)),
                _ => None,
            }
        }
        let mut c = Self {
            node,
            source_id: 0,
            destination_id: 0,
            property: b"",
            kind: CompactConnectionKind::Invalid,
        };
        if let (Some(tag), Some(source), Some(destination)) = (
            p.first().and_then(text),
            p.get(1).and_then(id),
            p.get(2).and_then(id),
        ) {
            c.source_id = source;
            c.destination_id = destination;
            match tag {
                b"OO" => c.kind = CompactConnectionKind::Object,
                b"OP" => {
                    if let Some(property) = p.get(3).and_then(text) {
                        c.kind = CompactConnectionKind::Property;
                        c.property = property;
                    }
                }
                _ => {}
            }
        }
        c
    }
}
#[derive(Default)]
pub(crate) struct DeferredData<'a> {
    input: &'a [u8],
    records: Vec<(usize, usize, usize)>,
    ordinals: Vec<usize>,
    pub(crate) connections: Vec<CompactConnection<'a>>,
}
impl<'a> DeferredData<'a> {
    pub(crate) fn ordinal(&self, node: usize) -> Result<usize, Error> {
        match self.ordinals.get(node) {
            Some(&ordinal) => Ok(ordinal),
            None => node
                .checked_add(1)
                .ok_or(Error::LimitExceeded { what: "nodes" }),
        }
    }
    pub(crate) fn restore(
        &self,
        node: usize,
        limits: &Limits,
    ) -> Result<Option<RestoredObject<'a>>, Error> {
        let Ok(i) = self.records.binary_search_by_key(&node, |r| r.0) else {
            return Ok(None);
        };
        let (_, start, end) = self.records[i];
        let (mut doc, _) = parse_binary_inner(self.input, limits, Some(start..end), false)?;
        // Retained-array interpretation depends on the Objects > AnimationCurve
        // ancestry, not just each child's spelling. Restore that context too.
        let mut used = doc
            .nodes
            .capacity()
            .checked_mul(size_of::<Node<'_>>())
            .and_then(|n| {
                doc.properties
                    .capacity()
                    .checked_mul(size_of::<Property<'_>>())
                    .and_then(|p| n.checked_add(p))
            })
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
        used = doc.properties.iter().try_fold(used, |n, p| {
            n.checked_add(owned_property_bytes(p)?)
                .ok_or(Error::LimitExceeded { what: "allocation" })
        })?;
        crate::allocation::reserve(&mut doc.nodes, 1, &mut used, limits.max_allocation)
            .map_err(allocation_error)?;
        for node in &mut doc.nodes {
            node.parent = Some(match node.parent {
                Some(p) => p
                    .checked_add(1)
                    .ok_or(Error::LimitExceeded { what: "nodes" })?,
                None => 0,
            });
        }
        doc.nodes.insert(
            0,
            Node {
                name: b"Objects",
                parent: None,
                properties: 0..0,
            },
        );
        let bytes = doc
            .nodes
            .capacity()
            .checked_mul(size_of::<Node<'_>>())
            .and_then(|n| {
                doc.properties
                    .capacity()
                    .checked_mul(size_of::<Property<'_>>())
                    .and_then(|p| n.checked_add(p))
            })
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
        let bytes = doc.properties.iter().try_fold(bytes, |n, p| {
            n.checked_add(owned_property_bytes(p)?)
                .ok_or(Error::LimitExceeded { what: "allocation" })
        })?;
        Ok(Some(RestoredObject {
            doc,
            _release: crate::allocation::Release::new(bytes),
        }))
    }
}
pub(crate) struct RestoredObject<'a> {
    doc: Document<'a>,
    _release: crate::allocation::Release,
}
impl RestoredObject<'_> {
    pub(crate) fn object_node(&self) -> usize {
        1
    }
}
impl<'a> std::ops::Deref for RestoredObject<'a> {
    type Target = Document<'a>;
    fn deref(&self) -> &Self::Target {
        &self.doc
    }
}
pub(crate) fn parse_lean<'a>(
    input: &'a [u8],
    limits: &Limits,
) -> Result<(Document<'a>, DeferredData<'a>), Error> {
    if input.starts_with(b"Kaydara FBX Binary  \x00\x1a") {
        parse_binary_inner(input, limits, None, true)
    } else {
        parse(input, limits).map(|doc| (doc, DeferredData::default()))
    }
}
fn parse_binary<'a>(input: &'a [u8], limits: &Limits) -> Result<Document<'a>, Error> {
    parse_binary_inner(input, limits, None, false).map(|(doc, _)| doc)
}
fn owned_property_bytes(p: &Property<'_>) -> Result<usize, Error> {
    Ok(match p {
        Property::String(Cow::Owned(v))
        | Property::Array {
            bytes: Cow::Owned(v),
            ..
        } => v.capacity(),
        Property::AsciiArray(v) => v
            .capacity()
            .checked_mul(size_of::<AsciiValue<'_>>())
            .ok_or(Error::LimitExceeded { what: "allocation" })?,
        _ => 0,
    })
}
fn animation_child(value_node: bool, name: &[u8]) -> bool {
    if value_node {
        return matches!(name, b"Properties70" | b"Properties60" | b"P" | b"Property");
    }
    matches!(
        name,
        b"Default"
            | b"KeyVer"
            | b"KeyTime"
            | b"KeyValueFloat"
            | b"KeyAttrFlags"
            | b"KeyAttrDataFloat"
            | b"KeyAttrRefCount"
            | b"Pre-Extrapolation"
            | b"Post-Extrapolation"
            | b"Type"
            | b"Repetition"
    )
}
// Header-only pass: exact retained/high-water capacities avoid geometric slack
// in the large DOM vectors. No arrays are decoded and source counts remain
// global even when a curve's temporary subtree is discarded.
#[derive(Default)]
struct Capacities {
    nodes: usize,
    properties: usize,
    objects: usize,
    connections: usize,
}
fn compact_capacities(
    input: &[u8],
    limits: &Limits,
    width: usize,
    be: bool,
) -> Result<Capacities, Error> {
    let mut r = Reader {
        bytes: input,
        pos: 27,
        be,
    };
    let mut stack: Vec<(usize, usize, &[u8])> = Vec::new();
    let mut total = 0usize;
    let (mut nodes, mut props, mut curves) = (0usize, 0usize, 0usize);
    let (mut seen_nodes, mut seen_props) = (0usize, 0usize);
    let (mut peak_nodes, mut peak_props) = (0usize, 0usize);
    let mut pending: Option<(usize, usize, bool, bool)> = None;
    let mut connections = 0usize;
    loop {
        let end = r.usize(width)?;
        let count = r.usize(width)?;
        let bytes = r.usize(width)?;
        let name_len = r.usize(1)?;
        if end == 0 {
            if count != 0 || bytes != 0 || name_len != 0 {
                return Err(Error::InvalidFormat);
            }
            let Some((parent, parent_end, _)) = stack.pop() else {
                break;
            };
            if r.pos != parent_end {
                return Err(Error::InvalidFormat);
            }
            if let Some((node, prop_end, eligible, _)) = pending {
                if node == parent {
                    if eligible {
                        nodes = node
                            .checked_add(1)
                            .ok_or(Error::LimitExceeded { what: "nodes" })?;
                        props = prop_end;
                        curves = curves
                            .checked_add(1)
                            .ok_or(Error::LimitExceeded { what: "nodes" })?;
                    }
                    pending = None;
                }
            }
            continue;
        }
        if end > input.len() {
            return Err(Error::Truncated);
        }
        if end <= r.pos || stack.last().is_some_and(|p| end > p.1) {
            return Err(Error::InvalidFormat);
        }
        if stack.len() >= limits.max_depth {
            return Err(Error::TooDeep);
        }
        seen_nodes = seen_nodes
            .checked_add(1)
            .filter(|&n| n <= limits.max_nodes)
            .ok_or(Error::LimitExceeded { what: "nodes" })?;
        seen_props = seen_props
            .checked_add(count)
            .filter(|&n| n <= limits.max_properties)
            .ok_or(Error::LimitExceeded { what: "properties" })?;
        if name_len > limits.max_string_length {
            return Err(Error::LimitExceeded {
                what: "string length",
            });
        }
        let name = r.take(name_len)?;
        let prop_end = r.pos.checked_add(bytes).ok_or(Error::Truncated)?;
        if prop_end > end {
            return Err(Error::InvalidFormat);
        }
        r.pos = prop_end;
        if let Some(ref mut p) = pending {
            p.2 &= animation_child(p.3, name);
        }
        if matches!(name, b"C" | b"Connect")
            && stack.len() == 1
            && stack[0].2 == b"Connections"
            && r.pos == end
        {
            peak_props = peak_props.max(
                props
                    .checked_add(count)
                    .ok_or(Error::LimitExceeded { what: "properties" })?,
            );
            connections = connections
                .checked_add(1)
                .ok_or(Error::LimitExceeded { what: "nodes" })?;
            continue;
        }
        let index = nodes;
        nodes = nodes
            .checked_add(1)
            .ok_or(Error::LimitExceeded { what: "nodes" })?;
        props = props
            .checked_add(count)
            .ok_or(Error::LimitExceeded { what: "properties" })?;
        peak_nodes = peak_nodes.max(nodes);
        peak_props = peak_props.max(props);
        if r.pos < end {
            if matches!(name, b"AnimationCurve" | b"AnimationCurveNode")
                && stack.len() == 1
                && stack[0].2 == b"Objects"
            {
                pending = Some((index, props, true, name == b"AnimationCurveNode"));
            }
            push(
                &mut stack,
                (index, end, name),
                &mut total,
                limits.max_allocation,
            )?;
        }
    }
    let bytes = stack
        .capacity()
        .checked_mul(size_of::<(usize, usize, &[u8])>())
        .ok_or(Error::LimitExceeded { what: "allocation" })?;
    drop(stack);
    crate::allocation::release_live(bytes);
    Ok(Capacities {
        nodes: peak_nodes,
        properties: peak_props,
        objects: curves,
        connections,
    })
}
fn parse_binary_inner<'a>(
    input: &'a [u8],
    limits: &Limits,
    region: Option<std::ops::Range<usize>>,
    defer: bool,
) -> Result<(Document<'a>, DeferredData<'a>), Error> {
    const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a";
    if !input.starts_with(MAGIC) {
        return Err(Error::InvalidFormat);
    }
    let endian = *input.get(22).ok_or(Error::Truncated)?;
    if endian > 1 {
        return Err(Error::InvalidFormat);
    }
    let mut r = Reader {
        bytes: input,
        pos: 23,
        be: endian == 1,
    };
    let version = u32::try_from(r.num(4)?).map_err(|_| Error::InvalidFormat)?;
    if !supported_version(version) && !(3000..MIN_VERSION).contains(&version) {
        return Err(Error::UnsupportedVersion { v: version });
    }
    // Node record headers use 64-bit words from 7500 on (ufbx and the
    // Blender format notes agree; 7700 keeps the 7500 layout).
    let width = if version >= 7500 { 8 } else { 4 };
    if let Some(ref region) = region {
        r.pos = region.start;
    }
    let mut deferred = DeferredData {
        input,
        records: Vec::new(),
        ordinals: Vec::new(),
        connections: Vec::new(),
    };
    // (retained node, end of its properties, source start, source end, eligible).
    let mut pending: Option<(usize, usize, usize, usize, bool, bool)> = None;
    let mut seen_nodes = 0usize;
    let mut seen_properties = 0usize;
    let mut doc = Document {
        version,
        nodes: Vec::new(),
        properties: Vec::new(),
    };
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut total = 0usize;
    if defer && version >= 7000 {
        let capacities = compact_capacities(input, limits, width, r.be)?;
        let nodes = capacities.nodes;
        let properties = capacities.properties;
        let curves = capacities.objects;
        crate::allocation::reserve(&mut doc.nodes, nodes, &mut total, limits.max_allocation)
            .map_err(allocation_error)?;
        crate::allocation::reserve(
            &mut doc.properties,
            properties,
            &mut total,
            limits.max_allocation,
        )
        .map_err(allocation_error)?;
        crate::allocation::reserve(
            &mut deferred.ordinals,
            nodes,
            &mut total,
            limits.max_allocation,
        )
        .map_err(allocation_error)?;
        crate::allocation::reserve(
            &mut deferred.records,
            curves,
            &mut total,
            limits.max_allocation,
        )
        .map_err(allocation_error)?;
        crate::allocation::reserve(
            &mut deferred.connections,
            capacities.connections,
            &mut total,
            limits.max_allocation,
        )
        .map_err(allocation_error)?;
    }
    let mut elements = 0usize;
    let mut decompressed = 0usize;
    loop {
        let source_start = r.pos;
        let end = r.usize(width)?;
        let count = r.usize(width)?;
        let property_bytes = r.usize(width)?;
        let name_len = r.usize(1)?;
        if end == 0 {
            if count != 0 || property_bytes != 0 || name_len != 0 {
                return Err(Error::InvalidFormat);
            }
            if let Some((parent, parent_end)) = stack.pop() {
                if r.pos != parent_end {
                    return Err(Error::InvalidFormat);
                }
                if let Some((node, prop_end, start, end, eligible, _)) = pending {
                    if node == parent {
                        if eligible {
                            let released =
                                doc.properties[prop_end..].iter().try_fold(0usize, |n, p| {
                                    n.checked_add(owned_property_bytes(p)?)
                                        .ok_or(Error::LimitExceeded { what: "allocation" })
                                })?;
                            doc.properties.truncate(prop_end);
                            doc.nodes.truncate(node + 1);
                            deferred.ordinals.truncate(node + 1);
                            total = total.checked_sub(released).ok_or(Error::InvalidFormat)?;
                            crate::allocation::release_live(released);
                            push(
                                &mut deferred.records,
                                (node, start, end),
                                &mut total,
                                limits.max_allocation,
                            )?;
                        }
                        pending = None;
                    }
                }
                if stack.is_empty() && region.as_ref().is_some_and(|region| r.pos == region.end) {
                    break;
                }
                continue;
            }
            break;
        }
        if end > input.len() {
            return Err(Error::Truncated);
        }
        if end <= r.pos
            || stack
                .last()
                .is_some_and(|(_, parent_end)| end > *parent_end)
        {
            return Err(Error::InvalidFormat);
        }
        if stack.len() >= limits.max_depth {
            return Err(Error::TooDeep);
        }
        if seen_nodes >= limits.max_nodes {
            return Err(Error::LimitExceeded { what: "nodes" });
        }
        if count > limits.max_properties.saturating_sub(seen_properties) {
            return Err(Error::LimitExceeded { what: "properties" });
        }
        if name_len > limits.max_string_length {
            return Err(Error::LimitExceeded {
                what: "string length",
            });
        }
        let name = r.take(name_len)?;
        seen_nodes = seen_nodes
            .checked_add(1)
            .ok_or(Error::LimitExceeded { what: "nodes" })?;
        seen_properties = seen_properties
            .checked_add(count)
            .ok_or(Error::LimitExceeded { what: "properties" })?;
        if let Some(ref mut curve) = pending {
            if !animation_child(curve.5, name) {
                curve.4 = false;
            }
        }
        let props_end = r.pos.checked_add(property_bytes).ok_or(Error::Truncated)?;
        if props_end > end {
            return Err(Error::InvalidFormat);
        }
        let start = doc.properties.len();
        for _ in 0..count {
            let kind = u8::try_from(r.num(1)?).map_err(|_| Error::InvalidFormat)?;
            let property = match kind {
                b'Y' => Property::I16(i16::from_ne_bytes(
                    u16::try_from(r.num(2)?)
                        .map_err(|_| Error::InvalidFormat)?
                        .to_ne_bytes(),
                )),
                b'C' => Property::Bool(u8::try_from(r.num(1)?).map_err(|_| Error::InvalidFormat)?),
                b'B' => {
                    Property::BoolAlt(u8::try_from(r.num(1)?).map_err(|_| Error::InvalidFormat)?)
                }
                b'I' => Property::I32(i32::from_ne_bytes(
                    u32::try_from(r.num(4)?)
                        .map_err(|_| Error::InvalidFormat)?
                        .to_ne_bytes(),
                )),
                b'F' => Property::F32(u32::try_from(r.num(4)?).map_err(|_| Error::InvalidFormat)?),
                b'D' => Property::F64(r.num(8)?),
                b'L' => Property::I64(i64::from_ne_bytes(r.num(8)?.to_ne_bytes())),
                b'S' | b'R' => {
                    let n = r.usize(4)?;
                    if n > limits.max_string_length {
                        return Err(Error::LimitExceeded {
                            what: "string length",
                        });
                    }
                    let b = r.take(n)?;
                    if kind == b'S' {
                        Property::String(Cow::Borrowed(b))
                    } else {
                        Property::Raw(b)
                    }
                }
                b'f' | b'd' | b'l' | b'i' | b'b' | b'c' => {
                    let count = r.usize(4)?;
                    elements = elements.checked_add(count).ok_or(Error::LimitExceeded {
                        what: "array elements",
                    })?;
                    if elements > limits.max_array_elements {
                        return Err(Error::LimitExceeded {
                            what: "array elements",
                        });
                    }
                    let encoding = u32::try_from(r.num(4)?).map_err(|_| Error::InvalidFormat)?;
                    let compressed_len = r.usize(4)?;
                    let width = match kind {
                        b'f' | b'i' => 4,
                        b'd' | b'l' => 8,
                        _ => 1,
                    };
                    let len = count.checked_mul(width).ok_or(Error::LimitExceeded {
                        what: "array bytes",
                    })?;
                    let source = r.take(compressed_len)?;
                    let bytes = match encoding {
                        0 if compressed_len == len => Cow::Borrowed(source),
                        0 => return Err(Error::InvalidFormat),
                        1 => {
                            decompressed =
                                decompressed.checked_add(len).ok_or(Error::LimitExceeded {
                                    what: "decompressed bytes",
                                })?;
                            if decompressed > limits.max_decompressed_bytes {
                                return Err(Error::LimitExceeded {
                                    what: "decompressed bytes",
                                });
                            }
                            let mut out = Vec::new();
                            crate::allocation::reserve(
                                &mut out,
                                len,
                                &mut total,
                                limits.max_allocation,
                            )
                            .map_err(allocation_error)?;
                            out.resize(len, 0);
                            let mut state = miniz_oxide::inflate::core::DecompressorOxide::new();
                            let (status, consumed, written) = miniz_oxide::inflate::core::decompress(&mut state, source, &mut out, 0,
                                miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF);
                            if status != miniz_oxide::inflate::TINFLStatus::Done
                                || consumed != source.len()
                                || written != len
                            {
                                return Err(Error::BadCompression);
                            }
                            Cow::Owned(out)
                        }
                        _ => return Err(Error::UnsupportedEncoding { encoding }),
                    };
                    Property::Array {
                        kind,
                        count,
                        big_endian: r.be,
                        bytes,
                    }
                }
                _ => return Err(Error::InvalidFormat),
            };
            if r.pos > props_end {
                return Err(Error::InvalidFormat);
            }
            push(
                &mut doc.properties,
                property,
                &mut total,
                limits.max_allocation,
            )?;
        }
        if r.pos != props_end {
            return Err(Error::InvalidFormat);
        }
        if defer
            && version >= 7000
            && matches!(name, b"C" | b"Connect")
            && stack.len() == 1
            && doc.nodes[stack[0].0].name == b"Connections"
            && r.pos == end
        {
            let node = seen_nodes.checked_sub(1).ok_or(Error::InvalidFormat)?;
            let record = CompactConnection::from_properties(node, &doc.properties[start..]);
            push(
                &mut deferred.connections,
                record,
                &mut total,
                limits.max_allocation,
            )?;
            let released = doc.properties[start..].iter().try_fold(0usize, |n, p| {
                n.checked_add(owned_property_bytes(p)?)
                    .ok_or(Error::LimitExceeded { what: "allocation" })
            })?;
            doc.properties.truncate(start);
            total = total.checked_sub(released).ok_or(Error::InvalidFormat)?;
            crate::allocation::release_live(released);
            continue;
        }
        let index = doc.nodes.len();
        push(
            &mut doc.nodes,
            Node {
                name,
                parent: stack.last().map(|x| x.0),
                properties: start..doc.properties.len(),
            },
            &mut total,
            limits.max_allocation,
        )?;
        if defer {
            push(
                &mut deferred.ordinals,
                seen_nodes,
                &mut total,
                limits.max_allocation,
            )?;
        }
        if r.pos < end {
            if defer
                && version >= 7000
                && matches!(name, b"AnimationCurve" | b"AnimationCurveNode")
                && stack.len() == 1
                && doc.nodes[stack[0].0].name == b"Objects"
            {
                pending = Some((
                    index,
                    doc.properties.len(),
                    source_start,
                    end,
                    true,
                    name == b"AnimationCurveNode",
                ));
            }
            push(&mut stack, (index, end), &mut total, limits.max_allocation)?;
        } else if r.pos != end {
            return Err(Error::InvalidFormat);
        } else if stack.is_empty() && region.as_ref().is_some_and(|region| r.pos == region.end) {
            break;
        }
    }
    if (3000..MIN_VERSION).contains(&version) {
        if let Some(header) = doc
            .nodes
            .iter()
            .position(|n| n.parent.is_none() && n.name == b"FBXHeaderExtension")
        {
            if let Some(node) = doc
                .nodes
                .iter()
                .find(|n| n.parent == Some(header) && n.name == b"FBXVersion")
            {
                let declared = match doc.properties.get(node.properties.start) {
                    Some(Property::I32(v)) => u32::try_from(*v).ok(),
                    Some(Property::I64(v)) => u32::try_from(*v).ok(),
                    _ => None,
                };
                if let Some(declared) = declared.filter(|&v| v > version && v < 6000) {
                    doc.version = declared;
                }
            }
        }
        if !supported_version(doc.version) {
            return Err(Error::UnsupportedVersion { v: doc.version });
        }
    }
    crate::allocation::release_live(
        stack
            .capacity()
            .checked_mul(size_of::<(usize, usize)>())
            .ok_or(Error::LimitExceeded { what: "allocation" })?,
    );
    Ok((doc, deferred))
}

#[cfg(test)]
mod deferred_tests {
    use super::*;

    struct Record {
        name: &'static str,
        count: u32,
        data: Vec<u8>,
        children: Vec<Record>,
    }
    fn record(name: &'static str, count: u32, data: Vec<u8>, children: Vec<Record>) -> Record {
        Record {
            name,
            count,
            data,
            children,
        }
    }
    fn emit(out: &mut Vec<u8>, node: &Record) {
        let start = out.len();
        out.resize(start + 13, 0);
        out.extend_from_slice(node.name.as_bytes());
        out.extend_from_slice(&node.data);
        for child in &node.children {
            emit(out, child);
        }
        if !node.children.is_empty() {
            out.extend_from_slice(&[0; 13]);
        }
        let end = u32::try_from(out.len()).unwrap();
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&node.count.to_le_bytes());
        out[start + 8..start + 12]
            .copy_from_slice(&u32::try_from(node.data.len()).unwrap().to_le_bytes());
        out[start + 12] = u8::try_from(node.name.len()).unwrap();
    }
    fn array(name: &'static str, kind: u8, bytes: &[u8], width: usize) -> Record {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6);
        let mut data = vec![kind];
        data.extend_from_slice(&u32::try_from(bytes.len() / width).unwrap().to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&u32::try_from(compressed.len()).unwrap().to_le_bytes());
        data.extend(compressed);
        record(name, 1, data, vec![])
    }
    fn fixture(custom: bool) -> Vec<u8> {
        let mut input = b"Kaydara FBX Binary  \x00\x1a\0".to_vec();
        input.extend_from_slice(&7400u32.to_le_bytes());
        let curves = (1..=2i64)
            .map(|id| {
                let mut props = vec![b'L'];
                props.extend_from_slice(&id.to_le_bytes());
                for name in [b"AnimCurve::C".as_slice(), b""] {
                    props.push(b'S');
                    props.extend_from_slice(&u32::try_from(name.len()).unwrap().to_le_bytes());
                    props.extend_from_slice(name);
                }
                let mut children = vec![
                    array("KeyTime", b'l', &[0; 16], 8),
                    array("KeyValueFloat", b'f', &[0; 8], 4),
                    array("KeyAttrFlags", b'i', &4u32.to_le_bytes(), 4),
                    array("KeyAttrDataFloat", b'f', &[0; 16], 4),
                    array("KeyAttrRefCount", b'i', &2u32.to_le_bytes(), 4),
                ];
                if custom {
                    children.push(record("Properties70", 0, vec![], vec![]));
                }
                record("AnimationCurve", 3, props, children)
            })
            .collect();
        emit(&mut input, &record("Objects", 0, vec![], curves));
        input.extend_from_slice(&[0; 13]);
        input
    }

    #[test]
    fn compact_curves_restore_exact_values_and_source_ordinals() {
        let input = fixture(false);
        let full = parse(&input, &Limits::default()).unwrap();
        let (compact, deferred) = parse_lean(&input, &Limits::default()).unwrap();
        assert_eq!(compact.nodes.len(), 3);
        assert_eq!(deferred.records.len(), 2);
        for node in 1..=2 {
            let ordinal = deferred.ordinal(node).unwrap();
            let restored = deferred.restore(node, &Limits::default()).unwrap().unwrap();
            let source = ordinal - 1;
            assert_eq!(full.nodes[source].name, restored.nodes[1].name);
            assert_eq!(
                full.properties[full.nodes[source].properties.clone()],
                restored.properties[restored.nodes[1].properties.clone()]
            );
            for (offset, restored_node) in restored.nodes.iter().enumerate().skip(2) {
                let original = &full.nodes[source + offset - 1];
                assert_eq!(original.name, restored_node.name);
                let a = crate::api_alloc::Budget::new(100_000);
                assert_eq!(
                    crate::canon_dom::animation_array(&full, source + offset - 1, false, &a)
                        .unwrap(),
                    crate::canon_dom::animation_array(&restored, offset, false, &a).unwrap(),
                );
                assert_eq!(
                    full.properties[original.properties.clone()],
                    restored.properties[restored_node.properties.clone()]
                );
            }
        }
        let public = crate::load(&input, &crate::LoadOptions::default()).unwrap();
        assert_eq!(public.animation_curves().len(), 2);
        assert_eq!(public.element_count(), 3);
        assert_eq!(public.animation_curves()[0].keys.len(), 2);
    }

    #[test]
    fn compact_parse_keeps_global_limits_and_compression_validation() {
        let input = fixture(false);
        let full = parse(&input, &Limits::default()).unwrap();
        for limits in [
            Limits {
                max_nodes: full.nodes.len() - 1,
                ..Limits::default()
            },
            Limits {
                max_properties: full.properties.len() - 1,
                ..Limits::default()
            },
            Limits {
                max_array_elements: 19,
                ..Limits::default()
            },
            Limits {
                max_decompressed_bytes: 87,
                ..Limits::default()
            },
        ] {
            assert!(parse(&input, &limits).is_err());
            assert!(parse_lean(&input, &limits).is_err());
        }
        let mut corrupt = input;
        let zlib = corrupt.windows(2).position(|b| b == [0x78, 0x9c]).unwrap();
        corrupt[zlib] ^= 1;
        assert!(matches!(
            parse_lean(&corrupt, &Limits::default()),
            Err(Error::BadCompression)
        ));
    }

    #[test]
    fn custom_curve_children_keep_the_complete_dom() {
        let input = fixture(true);
        let full = parse(&input, &Limits::default()).unwrap();
        let (compact, deferred) = parse_lean(&input, &Limits::default()).unwrap();
        assert_eq!(compact.nodes.len(), full.nodes.len());
        assert_eq!(compact.properties, full.properties);
        assert!(deferred.records.is_empty());
    }
    fn strings(values: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for s in values {
            out.push(b'S');
            out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_le_bytes());
            out.extend_from_slice(s);
        }
        out
    }
    fn object(
        kind: &'static str,
        id: i64,
        name: &[u8],
        subtype: &[u8],
        children: Vec<Record>,
    ) -> Record {
        let mut data = vec![b'L'];
        data.extend_from_slice(&id.to_le_bytes());
        data.extend(strings(&[name, subtype]));
        record(kind, 3, data, children)
    }
    fn connection(
        tag: &[u8],
        src: i64,
        dst: i64,
        property: &[u8],
        children: Vec<Record>,
    ) -> Record {
        let mut data = strings(&[tag]);
        for id in [src, dst] {
            data.push(b'L');
            data.extend_from_slice(&id.to_le_bytes());
        }
        let count = if tag == b"OP" {
            data.extend(strings(&[property]));
            4
        } else {
            3
        };
        record("C", count, data, children)
    }
    fn file(roots: Vec<Record>) -> Vec<u8> {
        let mut data = b"Kaydara FBX Binary  \x00\x1a\0".to_vec();
        data.extend_from_slice(&7400u32.to_le_bytes());
        for root in roots {
            emit(&mut data, &root);
        }
        data.extend_from_slice(&[0; 13]);
        data
    }
    fn models() -> Record {
        record(
            "Objects",
            0,
            vec![],
            vec![
                object("Model", 1, b"Model::Child", b"Null", vec![]),
                object("Model", 2, b"Model::Parent", b"Null", vec![]),
            ],
        )
    }
    fn property(name: &[u8], value: f64) -> Record {
        let mut data = strings(&[name, b"Number", b"", b"A"]);
        data.push(b'D');
        data.extend_from_slice(&value.to_bits().to_le_bytes());
        record("P", 5, data, vec![])
    }
    #[test]
    fn compact_value_properties_keep_authored_priority_and_template_fallback() {
        let definitions = record(
            "Definitions",
            0,
            vec![],
            vec![record(
                "ObjectType",
                1,
                strings(&[b"AnimationCurveNode"]),
                vec![record(
                    "PropertyTemplate",
                    1,
                    strings(&[b"FbxAnimCurveNode"]),
                    vec![record(
                        "Properties70",
                        0,
                        vec![],
                        vec![property(b"d|X", 3.0), property(b"d|Y", 9.0)],
                    )],
                )],
            )],
        );
        let input = file(vec![
            definitions,
            record(
                "Objects",
                0,
                vec![],
                vec![object(
                    "AnimationCurveNode",
                    3,
                    b"AnimCurveNode::T",
                    b"AnimCurveNode",
                    vec![record(
                        "Properties70",
                        0,
                        vec![],
                        vec![property(b"d|X", 42.0)],
                    )],
                )],
            ),
        ]);
        let full = crate::canon_scene::project(&input, &crate::canon_scene::DumpLimits::default())
            .unwrap();
        let public = crate::load(&input, &crate::LoadOptions::default()).unwrap();
        assert_eq!(
            public.animation_values()[0].default_value,
            full.animation.values[0].default_value
        );
        assert_eq!(public.animation_values()[0].default_value, [42.0, 9.0, 0.0]);
        let (doc, deferred) = parse_lean(&input, &Limits::default()).unwrap();
        let value = doc
            .nodes
            .iter()
            .position(|n| n.name == b"AnimationCurveNode")
            .unwrap();
        assert!(deferred
            .restore(value, &Limits::default())
            .unwrap()
            .is_some());
    }
    #[test]
    fn packed_connections_merge_with_unpacked_records_in_original_order() {
        let input = file(vec![
            models(),
            record(
                "Connections",
                0,
                vec![],
                vec![
                    connection(b"OO", 1, 2, b"", vec![]),
                    connection(
                        b"OP",
                        1,
                        2,
                        b"custom",
                        vec![record("Ignored", 0, vec![], vec![])],
                    ),
                    connection(b"OO", 2, 0, b"", vec![]),
                    connection(b"OP", 1, 2, b"", vec![]),
                ],
            ),
        ]);
        let full = parse(&input, &Limits::default()).unwrap();
        let (doc, deferred) = parse_lean(&input, &Limits::default()).unwrap();
        assert_eq!(deferred.connections.len(), 3);
        let limits = crate::scene::Limits::default();
        let a = crate::scene::index(&full, &limits).unwrap();
        let b = crate::scene::index_with(&doc, &limits, Some(&deferred)).unwrap();
        assert_eq!(
            a.objects
                .iter()
                .map(|o| (o.id, o.parent))
                .collect::<Vec<_>>(),
            b.objects
                .iter()
                .map(|o| (o.id, o.parent))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            a.connections
                .iter()
                .map(|c| (&c.kind, c.source, c.destination))
                .collect::<Vec<_>>(),
            b.connections
                .iter()
                .map(|c| (&c.kind, c.source, c.destination))
                .collect::<Vec<_>>()
        );
        let limited = crate::scene::Limits {
            max_connections: 2,
            ..limits
        };
        assert!(matches!(
            crate::scene::index_with(&doc, &limited, Some(&deferred)),
            Err(crate::scene::Error::LimitExceeded("connections"))
        ));
        for limits in [
            Limits {
                max_nodes: full.nodes.len() - 1,
                ..Limits::default()
            },
            Limits {
                max_properties: full.properties.len() - 1,
                ..Limits::default()
            },
        ] {
            assert!(parse_lean(&input, &limits).is_err());
        }
    }
    #[test]
    fn packed_connection_errors_preserve_original_validation_order_and_ordinals() {
        for ordinary_first in [false, true] {
            let ordinary = connection(b"XX", 1, 0, b"", vec![record("Ignored", 0, vec![], vec![])]);
            let compact = connection(b"OO", 999, 0, b"", vec![]);
            let records = if ordinary_first {
                vec![ordinary, compact]
            } else {
                vec![compact, ordinary]
            };
            let input = file(vec![models(), record("Connections", 0, vec![], records)]);
            let full = parse(&input, &Limits::default()).unwrap();
            let (doc, deferred) = parse_lean(&input, &Limits::default()).unwrap();
            let limits = crate::scene::Limits::default();
            let a = crate::scene::index(&full, &limits).unwrap_err();
            let b = crate::scene::index_with(&doc, &limits, Some(&deferred)).unwrap_err();
            assert_eq!(a, b);
        }
    }
}
