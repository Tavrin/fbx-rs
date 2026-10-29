//! Retained-DOM section of `fbx-scene-canon/1` from the Rust syntax DOM.
//!
//! Ports the projection qualified in reference/dom_equivalence.py, with
//! the parse context that the oracle chose taken from ufbx 0.23.1 (MIT,
//! <https://github.com/ufbx/ufbx/blob/v0.23.1/ufbx.c>): `ufbxi_update_parse_state`,
//! `ufbxi_is_array_node`, `ufbxi_is_raw_string`, `ufbxi_retain_dom_node`,
//! `ufbxi_push_sanitized_string` and the binary/ASCII array conversions.
use crate::syntax::{AsciiValue, Document, Property};
#[cfg(feature = "parity")]
use {
    crate::canon::{f64_into, hex, Emitter, Wr},
    sha2::{Digest, Sha256},
    std::fmt::Write as _,
    std::io::{self, Write},
};

const NUMBER: i64 = 0;
const STRING: i64 = 1;
const BLOB: i64 = 2;
const ARRAY_I32: i64 = 3;
const ARRAY_I64: i64 = 4;
const ARRAY_F32: i64 = 5;
const ARRAY_F64: i64 = 6;
const ARRAY_BLOB: i64 = 7;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Root,
    HeaderExt,
    SceneInfo,
    Thumbnail,
    Objects,
    Connections,
    Model,
    Geometry,
    NodeAttribute,
    AnimCurve,
    Deformer,
    AssociateModel,
    Pose,
    PoseNode,
    Texture,
    Video,
    LayeredTexture,
    SelectionNode,
    Audio,
    UnknownObject,
    Shape,
    LeNormal,
    LeBinormal,
    LeTangent,
    LeUv,
    LeColor,
    LeVertexCrease,
    LeEdgeCrease,
    LeSmoothing,
    LeVisibility,
    LePolygonGroup,
    LeHole,
    LeMaterial,
    LeOther,
    Takes,
    Take,
    TakeObject,
    Channel,
    Unknown,
}

/// `ufbxi_update_parse_state` (7.x subset; legacy states never occur in 7.x).
fn child_state(parent: St, name: &[u8]) -> St {
    use St::*;
    match parent {
        Root => match name {
            b"FBXHeaderExtension" => HeaderExt,
            b"Objects" => Objects,
            b"Connections" => Connections,
            b"Takes" => Takes,
            _ => Unknown,
        },
        HeaderExt if name == b"SceneInfo" => SceneInfo,
        SceneInfo if name == b"Thumbnail" => Thumbnail,
        Objects => match name {
            b"Model" => Model,
            b"Geometry" => Geometry,
            b"NodeAttribute" => NodeAttribute,
            b"AnimationCurve" => AnimCurve,
            b"Deformer" => Deformer,
            b"Pose" => Pose,
            b"Texture" => Texture,
            b"Video" => Video,
            b"LayeredTexture" => LayeredTexture,
            b"SelectionNode" => SelectionNode,
            b"Audio" => Audio,
            _ => UnknownObject,
        },
        Model | Geometry if name.first() == Some(&b'L') => match name {
            b"LayerElementNormal" => LeNormal,
            b"LayerElementBinormal" => LeBinormal,
            b"LayerElementTangent" => LeTangent,
            b"LayerElementUV" => LeUv,
            b"LayerElementColor" => LeColor,
            b"LayerElementVertexCrease" => LeVertexCrease,
            b"LayerElementEdgeCrease" => LeEdgeCrease,
            b"LayerElementSmoothing" => LeSmoothing,
            b"LayerElementVisibility" => LeVisibility,
            b"LayerElementPolygonGroup" => LePolygonGroup,
            b"LayerElementHole" => LeHole,
            b"LayerElementMaterial" => LeMaterial,
            _ if name.starts_with(b"LayerElement") => LeOther,
            _ => Unknown,
        },
        Model | Geometry if name == b"Shape" => Shape,
        Deformer if name == b"AssociateModel" => AssociateModel,
        Pose if name == b"PoseNode" => PoseNode,
        Takes if name == b"Take" => Take,
        Take => TakeObject,
        TakeObject | Channel if name == b"Channel" => Channel,
        _ => Unknown,
    }
}

/// `ufbxi_is_array_node` for the import profile options (retain_dom, nothing
/// ignored).
/// Returns the ufbx array element type character.
fn array_type(parent: St, name: &[u8], ascii: bool, version: u32) -> Option<u8> {
    use St::*;
    Some(match (parent, name) {
        (Thumbnail, b"ImageData") => b'c',
        (Geometry | Model, b"Vertices" | b"Points" | b"KnotVector" | b"KnotVectorU")
        | (Geometry | Model, b"KnotVectorV" | b"Normals") => b'd',
        (Geometry | Model, b"PolygonVertexIndex" | b"Edges" | b"Indexes" | b"PointsIndex") => b'i',
        (AnimCurve, b"KeyTime") => b'l',
        (AnimCurve, b"KeyValueFloat") => b'd',
        (AnimCurve, b"KeyAttrFlags" | b"KeyAttrRefCount") => b'i',
        (AnimCurve, b"KeyAttrDataFloat") => {
            if ascii && version >= 7200 {
                b'i'
            } else {
                b'f'
            }
        }
        (Texture, b"ModelUVTranslation" | b"ModelUVScaling" | b"Cropping") => b'd',
        (Video, b"Content") => b'C',
        (LayeredTexture, b"BlendModes") => b'i',
        (LayeredTexture, b"Alphas") => b'd',
        (SelectionNode, b"VertexIndexArray" | b"EdgeIndexArray" | b"PolygonIndexArray") => b'i',
        (LeNormal, b"Normals" | b"NormalsW")
        | (LeBinormal, b"Binormals" | b"BinormalsW")
        | (LeTangent, b"Tangents" | b"TangentsW")
        | (LeUv, b"UV")
        | (LeColor, b"Colors")
        | (LeVertexCrease, b"VertexCrease")
        | (LeEdgeCrease, b"EdgeCrease") => b'd',
        (LeNormal, b"NormalsIndex")
        | (LeBinormal, b"BinormalsIndex")
        | (LeTangent, b"TangentsIndex")
        | (LeUv, b"UVIndex")
        | (LeColor, b"ColorIndex")
        | (LeVertexCrease, b"VertexCreaseIndex") => b'i',
        (LeSmoothing, b"Smoothing") | (LeVisibility, b"Visibility") | (LeHole, b"Hole") => b'b',
        (LePolygonGroup, b"PolygonGroup") | (LeMaterial, b"Materials") => b'i',
        (LeOther, b"TextureId" | b"UVIndex") => b'i',
        (LeOther, b"UV") => b'd',
        (Shape, b"Indexes") => b'i',
        (Shape, b"Vertices" | b"Normals") => b'd',
        (Deformer, b"Transform" | b"TransformLink" | b"Weights" | b"BlendWeights")
        | (Deformer, b"FullWeights" | b"TransformAssociateModel") => b'd',
        (Deformer, b"Indexes") => b'i',
        (AssociateModel, b"Transform") => b'd',
        (PoseNode, b"Matrix") => b'd',
        (Channel, b"Key") => b'd',
        (Audio, b"Content" | b"BinaryData") => b'C',
        _ => return None,
    })
}

/// `ufbxi_is_raw_string` (7.x).
fn raw_string(parent: St, name: &[u8]) -> bool {
    use St::*;
    match parent {
        Root => name == b"Model" || name == b"FileId",
        HeaderExt => name == b"SceneInfo",
        Objects => true,
        Model | Geometry | NodeAttribute | UnknownObject => {
            name == b"NodeAttributeName" || name == b"Name"
        }
        Video => name == b"Content",
        Texture => name == b"TextureName" || name == b"Media",
        PoseNode | SelectionNode => name == b"Node",
        _ => false,
    }
}

fn utf8_step(b: &[u8], i: usize) -> usize {
    let c = u32::from(b[i]);
    let left = b.len() - i;
    let at = |k: usize| u32::from(b[i + k]);
    if c & 0x80 == 0 {
        usize::from(c != 0)
    } else if c & 0xe0 == 0xc0 && left >= 2 {
        let code = c << 8 | at(1);
        if code & 0xc0 == 0x80 && code >= 0xc280 {
            2
        } else {
            0
        }
    } else if c & 0xf0 == 0xe0 && left >= 3 {
        let code = c << 16 | at(1) << 8 | at(2);
        if code & 0xc0c0 == 0x8080 && code >= 0xe0a080 && !(0xeda080..0xee8080).contains(&code) {
            3
        } else {
            0
        }
    } else if c & 0xf8 == 0xf0 && left >= 4 {
        let code = c << 24 | at(1) << 16 | at(2) << 8 | at(3);
        if code & 0xc0c0c0 == 0x808080 && (0xf0908080..=0xf48fbfbf).contains(&code) {
            4
        } else {
            0
        }
    } else {
        0
    }
}
/// The string view ufbx retains for `raw` source bytes in a given context.
/// In a raw context ufbx 0.23.1 keeps no UTF-8 view at all (empty string).
pub(crate) fn string_view(raw: &[u8], raw_context: bool) -> Vec<u8> {
    if raw_context {
        return Vec::new();
    }
    if raw.iter().all(|&c| c > 0 && c < 0x80) {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let step = utf8_step(raw, i);
        if step > 0 {
            out.extend_from_slice(&raw[i..i + step]);
            i += step;
        } else {
            out.extend_from_slice(b"\xef\xbf\xbd");
            i += 1;
        }
    }
    out
}

pub(crate) fn f64_to_i64(x: f64) -> i64 {
    if x.is_nan() {
        i64::MIN
    } else if x.abs() <= 9_223_372_036_854_775_807.0 {
        if x >= 9_223_372_036_854_775_808.0 {
            i64::MIN
        } else {
            x as i64
        }
    } else if x >= 0.0 {
        i64::MAX
    } else {
        i64::MIN
    }
}
fn f64_to_i32(x: f64) -> i32 {
    if x.is_nan() {
        i32::MIN
    } else if x.abs() <= 2_147_483_647.0 {
        x as i32
    } else if x >= 0.0 {
        i32::MAX
    } else {
        i32::MIN
    }
}
fn to_u8(x: f64) -> u8 {
    if x.is_nan() || x.abs() >= 2_147_483_648.0 {
        0
    } else {
        (x as i64 & 0xff) as u8
    }
}

/// One retained DOM value.
enum DomValue {
    Number(i64, f64),
    Str(Vec<u8>, Vec<u8>),
    /// Typed numeric array: type and little-endian element bytes.
    Array(i64, usize, Vec<u8>),
    /// Byte array ('c'/'b'): stored as a BLOB.
    Bytes(Vec<u8>),
    Strings(Vec<Vec<u8>>),
}

#[derive(Clone, Copy)]
enum Tok {
    Int(i64),
    Float(f64),
    NegZero,
    Word(i64),
}
fn word(w: &[u8]) -> Tok {
    Tok::Word(w.first().map_or(0, |&b| i64::from(b as i8)))
}
fn ascii_tokens(p: &Property<'_>, out: &mut Vec<Tok>) {
    match p {
        Property::I64(v) => out.push(Tok::Int(*v)),
        Property::F64(bits) => out.push(Tok::Float(f64::from_bits(*bits))),
        Property::AsciiNegativeZero => out.push(Tok::NegZero),
        Property::AsciiBare(w) => out.push(word(w)),
        Property::AsciiArray(values) => {
            for v in values {
                out.push(match v {
                    AsciiValue::I64(v) => Tok::Int(*v),
                    AsciiValue::F64(bits) => Tok::Float(f64::from_bits(*bits)),
                    AsciiValue::NegativeZero => Tok::NegZero,
                    AsciiValue::Bare(w) => word(w),
                });
            }
        }
        _ => {}
    }
}
fn scalar_from_token(t: Tok) -> DomValue {
    match t {
        Tok::Int(v) | Tok::Word(v) => DomValue::Number(v, v as f64),
        Tok::NegZero => DomValue::Number(0, -0.0),
        Tok::Float(x) => DomValue::Number(f64_to_i64(x), x),
    }
}

/// Elements of a numeric target type, as f64/i64 pairs before encoding.
#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}
fn encode(target: u8, values: &[Num], boolean: bool) -> DomValue {
    let mut bytes = Vec::new();
    for &v in values {
        match target {
            b'c' | b'b' => bytes.push(match (v, boolean) {
                (Num::I(i), true) => u8::from(i != 0),
                (Num::F(f), true) => u8::from(f != 0.0),
                (Num::I(i), false) => i as u8,
                (Num::F(f), false) => to_u8(f),
            }),
            b'i' => bytes.extend_from_slice(&num_i32(v).to_le_bytes()),
            b'l' => bytes.extend_from_slice(
                &match v {
                    Num::I(i) => i,
                    Num::F(f) => f64_to_i64(f),
                }
                .to_le_bytes(),
            ),
            b'f' => bytes.extend_from_slice(
                &match v {
                    Num::I(i) => i as f64 as f32,
                    Num::F(f) => f as f32,
                }
                .to_le_bytes(),
            ),
            _ => bytes.extend_from_slice(
                &match v {
                    Num::I(i) => i as f64,
                    Num::F(f) => f,
                }
                .to_le_bytes(),
            ),
        }
    }
    let ty = match target {
        b'c' | b'b' => return DomValue::Bytes(bytes),
        b'i' => ARRAY_I32,
        b'l' => ARRAY_I64,
        b'f' => ARRAY_F32,
        _ => ARRAY_F64,
    };
    DomValue::Array(ty, values.len(), bytes)
}

/// Element width of a binary array kind, after checking the byte length.
fn binary_width(kind: u8, count: usize, bytes: &[u8]) -> Option<usize> {
    let width = match kind {
        b'b' | b'c' => 1,
        b'i' | b'f' => 4,
        b'l' | b'd' => 8,
        _ => return None,
    };
    (bytes.len() == count.checked_mul(width)?).then_some(width)
}

/// One binary array element (file byte order respected).
fn binary_num(kind: u8, big_endian: bool, c: &[u8]) -> Num {
    let width = c.len();
    let mut w = [0u8; 8];
    if big_endian {
        w[8 - width..].copy_from_slice(c);
        w.reverse();
    } else {
        w[..width].copy_from_slice(c);
    }
    let u = u64::from_le_bytes(w);
    match kind {
        b'b' | b'c' => Num::I(i64::from(u as u8 as i8)),
        b'i' => Num::I(i64::from(u as u32 as i32)),
        b'l' => Num::I(u as i64),
        b'f' => Num::F(f64::from(f32::from_bits(u as u32))),
        _ => Num::F(f64::from_bits(u)),
    }
}

/// Decode a binary array property into numbers (file byte order respected).
fn binary_array(kind: u8, count: usize, big_endian: bool, bytes: &[u8]) -> Option<Vec<Num>> {
    let width = binary_width(kind, count, bytes)?;
    Some(
        bytes
            .chunks_exact(width)
            .map(|c| binary_num(kind, big_endian, c))
            .collect(),
    )
}

/// [`encode`]'s element conversion for an `ARRAY_F64` target.
fn num_f64(v: Num) -> f64 {
    match v {
        Num::I(i) => i as f64,
        Num::F(f) => f,
    }
}

/// [`encode`]'s element conversion for an `ARRAY_I32` target (`'i'`).
fn num_i32(v: Num) -> i32 {
    match v {
        // Invalid wide integers must never alias a valid sparse index.
        Num::I(i) => i32::try_from(i).unwrap_or(i32::MIN),
        Num::F(f) => f64_to_i32(f),
    }
}

/// [`node_data`] for a binary-array node whose target type is `f64` or
/// `i32`: the same conversions as [`project`] then [`encode`], without the
/// intermediate number and byte vectors. `None` for other cases.
fn binary_array_data(doc: &Document<'_>, i: usize, target: u8) -> Option<NodeData> {
    let node = &doc.nodes[i];
    let Some(Property::Array {
        kind,
        count,
        big_endian,
        bytes,
    }) = doc.properties[node.properties.clone()].first()
    else {
        return None;
    };
    let (kind, be) = (*kind, *big_endian);
    let valid = binary_width(kind, *count, bytes).is_some();
    // Little-endian arrays already in the target type (the common case).
    match (target, kind, be, valid) {
        (b'i', b'i', false, true) => {
            let v = bytes.as_chunks::<4>().0.iter();
            return Some(NodeData::Ints(v.map(|c| i32::from_le_bytes(*c)).collect()));
        }
        (b'c' | b'b' | b'i' | b'l' | b'f' | b'C', _, _, _) => {}
        (_, b'd', false, true) => {
            let v = bytes.as_chunks::<8>().0.iter();
            return Some(NodeData::Reals(v.map(|c| f64::from_le_bytes(*c)).collect()));
        }
        (_, b'f', false, true) => {
            let v = bytes.as_chunks::<4>().0.iter();
            return Some(NodeData::Reals(
                v.map(|c| f64::from(f32::from_le_bytes(*c))).collect(),
            ));
        }
        _ => {}
    }
    let elements = || {
        let width = binary_width(kind, *count, bytes);
        bytes
            .chunks_exact(width.unwrap_or(1))
            .take(if width.is_some() { *count } else { 0 })
            .map(move |c| binary_num(kind, be, c))
    };
    match target {
        b'c' | b'b' | b'l' | b'f' | b'C' => None,
        b'i' => Some(NodeData::Ints(elements().map(num_i32).collect())),
        _ => Some(NodeData::Reals(elements().map(num_f64).collect())),
    }
}

fn project(doc: &Document<'_>, i: usize, _state: St, parent: St, ascii: bool) -> Vec<DomValue> {
    let node = &doc.nodes[i];
    let values = &doc.properties[node.properties.clone()];
    let name = node.name;
    if let Some(target) = array_type(parent, name, ascii, doc.version) {
        if target == b'C' {
            // ASCII embedded content is base64 that ufbx decodes while parsing
            // (`ufbxi_ascii_parse_node`), so the retained DOM holds the bytes.
            let strings = values
                .iter()
                .filter_map(|v| match v {
                    Property::String(s) if ascii => Some(decode_base64(s)),
                    Property::String(s) => Some(s.to_vec()),
                    Property::Raw(r) => Some(r.to_vec()),
                    _ => None,
                })
                .collect();
            return vec![DomValue::Strings(strings)];
        }
        let boolean = target == b'b';
        let mut nums = Vec::new();
        if !ascii {
            match values.first() {
                Some(Property::Array {
                    kind,
                    count,
                    big_endian,
                    bytes,
                }) => {
                    nums = binary_array(*kind, *count, *big_endian, bytes).unwrap_or_default();
                    if matches!(kind, b'b' | b'c') && matches!(target, b'b' | b'c') {
                        // Byte sources keep raw bytes (signed wrap) unless boolean.
                    }
                }
                _ => {
                    for v in values {
                        nums.push(match v {
                            Property::Bool(b) | Property::BoolAlt(b) => Num::I(i64::from(*b as i8)),
                            Property::I16(x) => Num::I(i64::from(*x)),
                            Property::I32(x) => Num::I(i64::from(*x)),
                            Property::I64(x) => Num::I(*x),
                            Property::F32(b) => Num::F(f64::from(f32::from_bits(*b))),
                            Property::F64(b) => Num::F(f64::from_bits(*b)),
                            _ => continue,
                        });
                    }
                }
            }
        } else {
            let accurate = name == b"KeyAttrDataFloat" && doc.version >= 7200;
            let mut toks = Vec::new();
            for v in values {
                ascii_tokens(v, &mut toks);
            }
            for t in toks {
                nums.push(match t {
                    Tok::NegZero if boolean => Num::I(0),
                    Tok::NegZero => match target {
                        b'f' | b'd' => Num::F(-0.0),
                        _ => Num::I(0),
                    },
                    Tok::Int(v) | Tok::Word(v) => Num::I(v),
                    Tok::Float(x) if accurate && target == b'i' => Num::F(f64::from(x as f32)),
                    Tok::Float(x) => Num::F(x),
                });
            }
        }
        return vec![encode(target, &nums, boolean)];
    }
    let mut out = Vec::new();
    let raw_context = raw_string(parent, name);
    for v in values {
        match v {
            Property::String(b) => out.push(DomValue::Str(string_view(b, raw_context), b.to_vec())),
            Property::Raw(b) => out.push(DomValue::Str(string_view(b, raw_context), b.to_vec())),
            Property::Bool(b) | Property::BoolAlt(b) => {
                let v = i64::from(*b as i8);
                out.push(DomValue::Number(v, v as f64))
            }
            Property::I16(x) => out.push(DomValue::Number(i64::from(*x), f64::from(*x))),
            Property::I32(x) => out.push(DomValue::Number(i64::from(*x), f64::from(*x))),
            Property::I64(x) if !ascii => out.push(DomValue::Number(*x, *x as f64)),
            Property::F32(b) => {
                let x = f64::from(f32::from_bits(*b));
                out.push(DomValue::Number(f64_to_i64(x), x))
            }
            Property::F64(b) if !ascii => {
                let x = f64::from_bits(*b);
                out.push(DomValue::Number(f64_to_i64(x), x))
            }
            Property::Array { .. } => break,
            other => {
                let mut toks = Vec::new();
                ascii_tokens(other, &mut toks);
                out.extend(toks.into_iter().map(scalar_from_token));
            }
        }
    }
    out.truncate(8);
    out
}

/// Parse state of DOM node `i` (`ufbxi_update_parse_state` along its ancestors).
fn state_of(doc: &Document<'_>, i: Option<usize>) -> St {
    let mut chain = Vec::new();
    let mut cur = i;
    while let Some(c) = cur {
        if chain.len() > 256 {
            return St::Unknown;
        }
        chain.push(c);
        cur = doc.nodes[c].parent;
    }
    chain
        .iter()
        .rev()
        .fold(St::Root, |s, &c| child_state(s, doc.nodes[c].name))
}

/// One scalar value of a retained node (`ufbxi_value`: number or string).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Scalar {
    /// Integer and floating views of a number.
    Num(i64, f64),
    /// Sanitized UTF-8 view of a string.
    Str(Vec<u8>),
}

/// Values of one source node as ufbx holds them after parsing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeData {
    /// `ufbx_real` array.
    Reals(Vec<f64>),
    /// `int32_t` array.
    Ints(Vec<i32>),
    /// Any other array type (int64, float, bytes, strings).
    OtherArray,
    /// Non-array values (first eight).
    Values(Vec<Scalar>),
}

/// Upper bound on numeric values, without decoding or allocating. Includes
/// scalar properties for binary nodes that do not use an array property.
pub(crate) fn data_len(doc: &Document<'_>, i: usize) -> Result<usize, String> {
    doc.properties[doc.nodes[i].properties.clone()]
        .iter()
        .try_fold(0usize, |n, p| {
            let count = match p {
                Property::Array { count, .. } => *count,
                Property::AsciiArray(v) => v.len(),
                _ => 1,
            };
            n.checked_add(count)
                .ok_or_else(|| "deformer: allocation limit".into())
        })
}

/// The converted values of DOM node `i` (`ufbxi_get_array` / `ufbxi_get_val*`),
/// shared with the retained-DOM projection so both see the same conversions.
pub(crate) fn node_data(doc: &Document<'_>, i: usize, ascii: bool) -> NodeData {
    let parent = state_of(doc, doc.nodes[i].parent);
    let state = child_state(parent, doc.nodes[i].name);
    let target = array_type(parent, doc.nodes[i].name, ascii, doc.version);
    // Numeric ASCII arrays are already tokenized by the syntax parser. Decode
    // them directly, using exactly the retained-DOM conversions, rather than
    // allocating token, number and encoded-byte vectors only to decode again.
    // KeyAttrDataFloat has a special float32 rounding rule and stays below.
    if ascii && doc.nodes[i].name != b"KeyAttrDataFloat" {
        if let [Property::AsciiArray(values)] = &doc.properties[doc.nodes[i].properties.clone()] {
            let nums = || {
                values.iter().map(|v| match v {
                    AsciiValue::I64(v) => Num::I(*v),
                    AsciiValue::F64(bits) => Num::F(f64::from_bits(*bits)),
                    AsciiValue::NegativeZero => Num::F(-0.0),
                    AsciiValue::Bare(w) => Num::I(w.first().map_or(0, |&b| i64::from(b as i8))),
                })
            };
            match target {
                Some(b'i') => return NodeData::Ints(nums().map(num_i32).collect()),
                Some(b'd') => return NodeData::Reals(nums().map(num_f64).collect()),
                _ => {}
            }
        }
    }
    if let (Some(t), false) = (target, ascii) {
        if let Some(data) = binary_array_data(doc, i, t) {
            return data;
        }
    }
    let values = project(doc, i, state, parent, ascii);
    if target.is_some() {
        return match values.into_iter().next() {
            Some(DomValue::Array(ARRAY_F64, _, bytes)) => NodeData::Reals(
                bytes
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| f64::from_le_bytes(*c))
                    .collect(),
            ),
            Some(DomValue::Array(ARRAY_I32, _, bytes)) => NodeData::Ints(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| i32::from_le_bytes(*c))
                    .collect(),
            ),
            _ => NodeData::OtherArray,
        };
    }
    NodeData::Values(
        values
            .into_iter()
            .filter_map(|v| match v {
                DomValue::Number(i, f) => Some(Scalar::Num(i, f)),
                DomValue::Str(view, _) => Some(Scalar::Str(view)),
                _ => None,
            })
            .collect(),
    )
}

/// `ufbxi_decode_base64`: 4-character groups through ufbx's table (invalid
/// characters map to 0x80, `=` to 0x40, both truncated into the output
/// bytes), then the trailing padding rule. Bad input only warns in ufbx.
fn decode_base64(src: &[u8]) -> Vec<u8> {
    let table = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 0x40,
            _ => 0x80,
        }
    };
    let mut out = Vec::with_capacity(src.len() / 4 * 3 + 3);
    for g in src.as_chunks::<4>().0 {
        let (a, b, c, d) = (table(g[0]), table(g[1]), table(g[2]), table(g[3]));
        out.push((a << 2 | b >> 4) as u8);
        out.push((b << 4 | c >> 2) as u8);
        out.push((c << 6 | d) as u8);
    }
    if src.len() >= 4 {
        let end = &src[src.len() - 4..];
        let mut padding = 0u32;
        for (k, bit) in [8, 4, 2, 1].into_iter().enumerate() {
            if end[k] == b'=' {
                padding |= bit;
            }
        }
        if padding <= 1 {
            out.truncate(out.len() - padding as usize);
        } else if padding == 3 {
            out.truncate(out.len() - 2);
        }
    }
    out
}

/// First string value of DOM node `i` as `(utf8 view, raw bytes)`, the
/// `"S"`/`"b"` views of `ufbxi_get_val1`. `None` when the first value is not a
/// string, or for the UTF-8 view in a raw-string context (no view retained).
pub(crate) fn node_string(
    doc: &Document<'_>,
    i: usize,
    ascii: bool,
) -> Option<(Option<Vec<u8>>, Vec<u8>)> {
    let parent = state_of(doc, doc.nodes[i].parent);
    let state = child_state(parent, doc.nodes[i].name);
    let raw_context = raw_string(parent, doc.nodes[i].name);
    match project(doc, i, state, parent, ascii).into_iter().next() {
        Some(DomValue::Str(view, raw)) => Some(((!raw_context).then_some(view), raw)),
        _ => None,
    }
}

/// `ufbxi_read_embedded_blob` over DOM node `i`: the 'C' array parts joined
/// (a single binary part is used as is). Empty when the node is no 'C' array.
pub(crate) fn node_content(doc: &Document<'_>, i: usize, ascii: bool) -> Vec<u8> {
    let parent = state_of(doc, doc.nodes[i].parent);
    if array_type(parent, doc.nodes[i].name, ascii, doc.version) != Some(b'C') {
        return Vec::new();
    }
    let state = child_state(parent, doc.nodes[i].name);
    match project(doc, i, state, parent, ascii).into_iter().next() {
        Some(DomValue::Strings(parts)) => parts.concat(),
        _ => Vec::new(),
    }
}

/// Boolean ('b') array of DOM node `i` as ufbx converts it, if it is one.
pub(crate) fn node_bools(doc: &Document<'_>, i: usize, ascii: bool) -> Option<Vec<bool>> {
    let parent = state_of(doc, doc.nodes[i].parent);
    if array_type(parent, doc.nodes[i].name, ascii, doc.version) != Some(b'b') {
        return None;
    }
    let state = child_state(parent, doc.nodes[i].name);
    match project(doc, i, state, parent, ascii).into_iter().next() {
        Some(DomValue::Bytes(b)) => Some(b.iter().map(|&x| x != 0).collect()),
        _ => None,
    }
}

#[cfg(feature = "parity")]
fn blob_value(b: &mut Vec<u8>, bytes: &[u8]) {
    let _ = write!(
        Wr(b),
        "{{\"type\":\"blob\",\"length\":\"{}\",\"sha256\":\"",
        bytes.len()
    );
    hex(b, &Sha256::digest(bytes));
    b.extend_from_slice(b"\"}");
}
#[cfg(feature = "parity")]
fn string_value(b: &mut Vec<u8>, bytes: &[u8]) {
    let _ = write!(Wr(b), "{{\"length\":\"{}\",\"bytes_hex\":\"", bytes.len());
    hex(b, bytes);
    b.extend_from_slice(b"\"}");
}

#[cfg(feature = "parity")]
fn write_value<W: Write>(em: &mut Emitter<'_, W>, v: &DomValue) -> io::Result<()> {
    let (ty, s, int, float): (i64, &[u8], i64, f64) = match v {
        DomValue::Number(i, f) => (NUMBER, b"", *i, *f),
        DomValue::Str(s, _) => (STRING, s, 0, 0.0),
        DomValue::Array(t, n, _) => (*t, b"", *n as i64, *n as f64),
        DomValue::Bytes(b) => (BLOB, b"", b.len() as i64, b.len() as f64),
        DomValue::Strings(v) => (ARRAY_BLOB, b"", v.len() as i64, v.len() as f64),
    };
    em.leaf_raw("type", |b| {
        let _ = write!(
            Wr(b),
            "{{\"enum\":\"ufbx_dom_value_type\",\"i64\":\"{ty}\"}}"
        );
    })?;
    em.leaf_raw("value_str", |b| string_value(b, s))?;
    em.leaf_raw("value_int", |b| {
        let _ = write!(Wr(b), "{{\"i64\":\"{int}\"}}");
    })?;
    em.leaf_raw("value_float", |b| f64_into(b, float))?;
    let m = em.push("array");
    match v {
        DomValue::Strings(list) => {
            em.count("length", list.len())?;
            for (c, chunk) in list.chunks(4096).enumerate() {
                em.leaf_raw(&(c * 4096).to_string(), |b| {
                    b.push(b'[');
                    for (k, s) in chunk.iter().enumerate() {
                        if k > 0 {
                            b.push(b',');
                        }
                        string_value(b, s);
                    }
                    b.push(b']');
                })?;
            }
        }
        DomValue::Array(t, n, bytes) => {
            em.count("length", *n)?;
            let width = if matches!(*t, ARRAY_I32 | ARRAY_F32) {
                4
            } else {
                8
            };
            for (c, chunk) in bytes.chunks(4096 * width).enumerate() {
                em.leaf_raw(&(c * 4096).to_string(), |b| {
                    b.push(b'[');
                    for (k, e) in chunk.chunks_exact(width).enumerate() {
                        if k > 0 {
                            b.push(b',');
                        }
                        let mut w = [0u8; 8];
                        w[..width].copy_from_slice(e);
                        let u = u64::from_le_bytes(w);
                        let _ = match *t {
                            ARRAY_I32 => write!(Wr(b), "{{\"i64\":\"{}\"}}", u as u32 as i32),
                            ARRAY_I64 => write!(Wr(b), "{{\"i64\":\"{}\"}}", u as i64),
                            ARRAY_F32 => write!(Wr(b), "{{\"f32\":\"{:08x}\"}}", u as u32),
                            _ => write!(Wr(b), "{{\"f64\":\"{u:016x}\"}}"),
                        };
                    }
                    b.push(b']');
                })?;
            }
        }
        _ => {
            let blob: &[u8] = match v {
                DomValue::Str(_, raw) => raw,
                DomValue::Bytes(b) => b,
                _ => b"",
            };
            em.leaf_raw_here(|b| blob_value(b, blob))?;
            for (c, chunk) in blob.chunks(4096).enumerate() {
                em.leaf_raw(&(c * 4096).to_string(), |b| {
                    b.extend_from_slice(b"{\"bytes_hex\":\"");
                    hex(b, chunk);
                    b.extend_from_slice(b"\"}");
                })?;
            }
        }
    }
    em.restore(m);
    Ok(())
}

#[cfg(feature = "parity")]
#[allow(clippy::too_many_arguments)]
fn emit_node<W: Write>(
    em: &mut Emitter<'_, W>,
    doc: &Document<'_>,
    children: &[Vec<usize>],
    o: usize,
    state: St,
    parent: St,
    ascii: bool,
    depth: usize,
) -> io::Result<()> {
    if depth > 256 {
        return Err(io::Error::other("DOM serialization depth"));
    }
    let name: &[u8] = if o == 0 { b"" } else { doc.nodes[o - 1].name };
    if name == b"CreationTimeStamp" {
        return Ok(());
    }
    em.count("ordinal", o)?;
    em.leaf_raw("name", |b| string_value(b, name))?;
    let values = if o == 0 {
        Vec::new()
    } else {
        project(doc, o - 1, state, parent, ascii)
    };
    let m = em.push("values");
    em.count("length", values.len())?;
    for (k, v) in values.iter().enumerate() {
        let mk = em.push_index(k);
        write_value(em, v)?;
        em.restore(mk);
    }
    em.restore(m);
    let m = em.push("children");
    em.count("length", children[o].len())?;
    for (k, &c) in children[o].iter().enumerate() {
        let mk = em.push_index(k);
        let child_name = doc.nodes[c - 1].name;
        emit_node(
            em,
            doc,
            children,
            c,
            child_state(state, child_name),
            state,
            ascii,
            depth + 1,
        )?;
        em.restore(mk);
    }
    em.restore(m);
    Ok(())
}

/// Write the `dom` section: preorder nodes, CreationTimeStamp subtrees omitted.
#[cfg(feature = "parity")]
pub fn write_dom<W: Write>(
    em: &mut Emitter<'_, W>,
    doc: &Document<'_>,
    ascii: bool,
) -> io::Result<()> {
    em.begin("dom")?;
    let n = doc.nodes.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for (i, node) in doc.nodes.iter().enumerate() {
        children[node.parent.map_or(0, |p| p + 1)].push(i + 1);
    }
    emit_node(em, doc, &children, 0, St::Root, St::Root, ascii, 0)?;
    em.end()
}

/// Numeric animation arrays in their retained byte representation.
pub(crate) fn animation_array(doc: &Document<'_>, i: usize, ascii: bool) -> Option<(i64, Vec<u8>)> {
    let parent = state_of(doc, doc.nodes[i].parent);
    let state = child_state(parent, doc.nodes[i].name);
    match project(doc, i, state, parent, ascii).into_iter().next()? {
        DomValue::Array(kind, _, bytes) => Some((kind, bytes)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_ascii_arrays_match_retained_projection_bits() {
        let input = b"; FBX 7.4.0\nObjects: { Geometry: 1, \"Geometry::S\", \"Shape\" {\nIndexes: *6 { a: 0,-1,2147483647,4294967296,1.5,-0 }\nVertices: *6 { a: -0,1.23456789,42,-3.25,1e-100,T }\n}\n}\n";
        let doc = crate::syntax::parse(input, &Default::default()).unwrap();
        for (i, n) in doc.nodes.iter().enumerate() {
            if !matches!(n.name, b"Indexes" | b"Vertices") {
                continue;
            }
            let parent = state_of(&doc, n.parent);
            let values = project(&doc, i, child_state(parent, n.name), parent, true);
            match (node_data(&doc, i, true), &values[0]) {
                (NodeData::Ints(v), DomValue::Array(ARRAY_I32, _, bytes)) => {
                    assert_eq!(
                        v,
                        bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| i32::from_le_bytes(*c))
                            .collect::<Vec<_>>()
                    );
                }
                (NodeData::Reals(v), DomValue::Array(ARRAY_F64, _, bytes)) => {
                    assert_eq!(
                        v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        bytes
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| u64::from_le_bytes(*c))
                            .collect::<Vec<_>>()
                    );
                }
                _ => panic!("numeric array"),
            }
        }
    }

    #[test]
    fn ascii_content_base64_matches_ufbx_padding_rules() {
        assert_eq!(decode_base64(b"TWVzaA=="), b"Mesh");
        assert_eq!(decode_base64(b"TWVzaC4="), b"Mesh.");
        assert_eq!(decode_base64(b"TWVzaC4h"), b"Mesh.!");
        // A trailing partial group is dropped; ufbx only warns.
        assert_eq!(decode_base64(b"TWVzaC4hTQ"), b"Mesh.!");
    }
}
