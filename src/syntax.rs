//! FBX 7.x syntax layer: binary and ASCII documents into a bounded flat DOM.
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
            max_nodes: 1 << 20,
            max_properties: 1 << 22,
            max_array_elements: 1 << 26,
            max_decompressed_bytes: 256 << 20,
            max_string_length: 16 << 20,
            max_allocation: 512 << 20,
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
    /// The file version is outside [`MIN_VERSION`]..=[`MAX_VERSION`].
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
fn charge(total: &mut usize, amount: usize, limit: usize) -> Result<(), Error> {
    *total = total
        .checked_add(amount)
        .ok_or(Error::LimitExceeded { what: "allocation" })?;
    if *total > limit {
        return Err(Error::LimitExceeded { what: "allocation" });
    }
    Ok(())
}
fn push<T>(vec: &mut Vec<T>, value: T, total: &mut usize, limit: usize) -> Result<(), Error> {
    if vec.len() == vec.capacity() {
        // A growth may temporarily keep both old and new allocations live.
        let old = vec
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
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
        let requested = target
            .checked_mul(size_of::<T>())
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
        charge(total, requested, limit)?;
        vec.try_reserve_exact(target - vec.len())
            .map_err(|_| Error::AllocationFailed)?;
        let actual = vec
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or(Error::LimitExceeded { what: "allocation" })?;
        if actual > requested {
            charge(total, actual - requested, limit)?;
        }
        *total -= old;
    }
    vec.push(value);
    Ok(())
}
/// Oldest accepted FBX version (7.1).
pub const MIN_VERSION: u32 = 7100;
/// Newest accepted FBX version (7.7).
pub const MAX_VERSION: u32 = 7700;

/// Whether `version` is in the accepted FBX 7.x range (7100..=7700).
/// 6.1 and older use a different document model and are rejected.
pub fn supported_version(version: u32) -> bool {
    (MIN_VERSION..=MAX_VERSION).contains(&version)
}

/// Parse a binary or ASCII FBX 7.x document (7100..=7700).
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

fn parse_binary<'a>(input: &'a [u8], limits: &Limits) -> Result<Document<'a>, Error> {
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
    let version = r.num(4)? as u32;
    if !supported_version(version) {
        return Err(Error::UnsupportedVersion { v: version });
    }
    // Node record headers use 64-bit words from 7500 on (ufbx and the
    // Blender format notes agree; 7700 keeps the 7500 layout).
    let width = if version >= 7500 { 8 } else { 4 };
    let mut doc = Document {
        version,
        nodes: Vec::new(),
        properties: Vec::new(),
    };
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut total = 0usize;
    let mut elements = 0usize;
    let mut decompressed = 0usize;
    loop {
        let end = r.usize(width)?;
        let count = r.usize(width)?;
        let property_bytes = r.usize(width)?;
        let name_len = r.usize(1)?;
        if end == 0 {
            if count != 0 || property_bytes != 0 || name_len != 0 {
                return Err(Error::InvalidFormat);
            }
            if let Some((_, parent_end)) = stack.pop() {
                if r.pos != parent_end {
                    return Err(Error::InvalidFormat);
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
        if doc.nodes.len() >= limits.max_nodes {
            return Err(Error::LimitExceeded { what: "nodes" });
        }
        if count > limits.max_properties.saturating_sub(doc.properties.len()) {
            return Err(Error::LimitExceeded { what: "properties" });
        }
        if name_len > limits.max_string_length {
            return Err(Error::LimitExceeded {
                what: "string length",
            });
        }
        let name = r.take(name_len)?;
        let props_end = r.pos.checked_add(property_bytes).ok_or(Error::Truncated)?;
        if props_end > end {
            return Err(Error::InvalidFormat);
        }
        let start = doc.properties.len();
        for _ in 0..count {
            let kind = r.num(1)? as u8;
            let property = match kind {
                b'Y' => Property::I16(r.num(2)? as i16),
                b'C' => Property::Bool(r.num(1)? as u8),
                b'B' => Property::BoolAlt(r.num(1)? as u8),
                b'I' => Property::I32(r.num(4)? as i32),
                b'F' => Property::F32(r.num(4)? as u32),
                b'D' => Property::F64(r.num(8)?),
                b'L' => Property::I64(r.num(8)? as i64),
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
                    let encoding = r.num(4)? as u32;
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
                            charge(&mut total, len, limits.max_allocation)?;
                            let mut out = Vec::new();
                            out.try_reserve_exact(len)
                                .map_err(|_| Error::AllocationFailed)?;
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
        if r.pos < end {
            push(&mut stack, (index, end), &mut total, limits.max_allocation)?;
        } else if r.pos != end {
            return Err(Error::InvalidFormat);
        }
    }
    Ok(doc)
}
