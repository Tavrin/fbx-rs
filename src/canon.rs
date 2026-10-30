//! Schema-driven `fbx-scene-canon/1` field emitter.
//!
//! The field order and leaf types come from `canon_schema.rs`, generated from
//! the pinned ufbx.h exactly as the C oracle's walker is (reference/generate.py
//! and reference/generate_rust.py). A projection supplies a value tree; any
//! field the tree omits is emitted as the type's zero value (false, 0, empty,
//! null), which is what a zero-initialised ufbx struct holds.
#[cfg(feature = "parity")]
use {
    crate::canonical_writer::CanonicalWriter,
    sha2::{Digest, Sha256},
    std::fmt::Write as _,
    std::io::{self, Write},
};

/// Leaf or aggregate kind of one generated schema field.
#[cfg(feature = "parity")]
#[derive(Debug, Clone, Copy)]
pub enum K {
    Bool,
    I64,
    U64,
    F32,
    F64,
    Enum(&'static str),
    Str,
    Blob,
    V2,
    V3,
    V4,
    Quat,
    Mat,
    Ref,
    DomRef,
    /// Dense handle replaced by source identity; `true` for node handles.
    #[allow(dead_code)]
    Handle(bool),
    Present,
    S(&'static str),
    Opt(&'static str),
    Arr(&'static K, usize),
}
/// One generated field.
#[cfg(feature = "parity")]
pub(crate) struct F {
    pub name: &'static str,
    pub kind: K,
}
/// Lists whose canonical order is a sorted copy (canon.c `stable_sorted_copy`).
#[cfg(feature = "parity")]
#[derive(Debug, Clone, Copy)]
pub(crate) enum Sorter {
    Connection,
    AnimProp,
    PropOverride,
    TransformOverride,
}
#[cfg(feature = "parity")]
pub(crate) enum Body {
    Fields {
        named_len: Option<usize>,
        fields: &'static [F],
    },
    List {
        item: K,
        sorter: Option<Sorter>,
    },
}

/// A projected value. Aggregates are field lists in any order; lookups are by name.
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Bool(bool),
    I(i64),
    U(u64),
    F32(f32),
    F(f64),
    Bytes(Vec<u8>),
    Vec(Vec<f64>),
    Ref(Option<usize>),
    Dom(Option<usize>),
    Obj(Obj),
    Null,
    List(Vec<V>),
    F64s(Vec<f64>),
    U32s(Vec<u32>),
    I32s(Vec<i32>),
    Vec2s(Vec<[f64; 2]>),
    Vec3s(Vec<[f64; 3]>),
    Vec4s(Vec<[f64; 4]>),
    Bools(Vec<bool>),
}
/// Named fields of one projected struct.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Obj(pub Vec<(&'static str, V)>);
impl Obj {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn set(&mut self, name: &'static str, v: V) -> &mut Self {
        if let Some(slot) = self.0.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = v;
        } else {
            self.0.push((name, v));
        }
        self
    }
    pub fn with(mut self, name: &'static str, v: V) -> Self {
        self.set(name, v);
        self
    }
    pub(crate) fn try_set(
        &mut self,
        name: &'static str,
        v: V,
        a: &crate::api_alloc::Budget,
    ) -> Result<&mut Self, crate::Error> {
        if let Some(slot) = self.0.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = v;
        } else {
            a.push(&mut self.0, (name, v))?;
        }
        Ok(self)
    }
    pub(crate) fn try_with(
        mut self,
        name: &'static str,
        v: V,
        a: &crate::api_alloc::Budget,
    ) -> Result<Self, crate::Error> {
        self.try_set(name, v, a)?;
        Ok(self)
    }
    pub fn get(&self, name: &str) -> Option<&V> {
        self.0.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, name: &str) -> Option<&mut V> {
        self.0.iter_mut().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
}
pub fn s(bytes: &[u8]) -> V {
    V::Bytes(bytes.to_vec())
}
pub fn v3(v: [f64; 3]) -> V {
    V::Vec(v.to_vec())
}

#[cfg(feature = "parity")]
fn lookup(name: &str) -> io::Result<&'static Body> {
    crate::canon_schema::STRUCTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| b)
        .ok_or_else(|| io::Error::other(format!("unknown schema struct {name}")))
}

/// Field walker state: element keys for references and the output stream.
#[cfg(feature = "parity")]
pub struct Emitter<'k, W: Write> {
    pub w: CanonicalWriter<W>,
    pub keys: &'k [String],
    path: String,
    buf: Vec<u8>,
    depth: usize,
}

#[cfg(feature = "parity")]
fn hex_into(out: &mut Vec<u8>, bytes: &[u8]) {
    const H: &[u8; 16] = b"0123456789abcdef";
    out.reserve(bytes.len() * 2);
    for &b in bytes {
        out.push(H[usize::from(b >> 4)]);
        out.push(H[usize::from(b & 15)]);
    }
}
#[cfg(feature = "parity")]
fn json_str(out: &mut Vec<u8>, s: &str) {
    // Keys and enum names are ASCII without quotes or controls.
    out.push(b'"');
    out.extend_from_slice(s.as_bytes());
    out.push(b'"');
}

#[cfg(feature = "parity")]
impl<'k, W: Write> Emitter<'k, W> {
    pub fn new(w: CanonicalWriter<W>, keys: &'k [String]) -> Self {
        Self {
            w,
            keys,
            path: String::new(),
            buf: Vec::new(),
            depth: 0,
        }
    }
    pub fn begin(&mut self, section: &str) -> io::Result<()> {
        self.w.begin(section)?;
        self.path.clear();
        self.path.push('/');
        self.path.push_str(section);
        Ok(())
    }
    pub fn end(&mut self) -> io::Result<()> {
        self.w.end()
    }
    pub fn push(&mut self, name: &str) -> usize {
        let mark = self.path.len();
        self.path.push('/');
        self.path.push_str(name);
        mark
    }
    pub fn push_index(&mut self, i: usize) -> usize {
        let mark = self.path.len();
        let _ = write!(self.path, "/{i}");
        mark
    }
    pub fn restore(&mut self, mark: usize) {
        self.path.truncate(mark);
    }
    fn flush_leaf(&mut self) -> io::Result<()> {
        let buf = std::mem::take(&mut self.buf);
        let r = self.w.field_raw(&self.path, &buf);
        self.buf = buf;
        self.buf.clear();
        r
    }
    /// Emit one leaf at `path/name` whose raw JSON is produced by `f`.
    pub fn leaf_raw(&mut self, name: &str, f: impl FnOnce(&mut Vec<u8>)) -> io::Result<()> {
        let m = self.push(name);
        self.buf.clear();
        f(&mut self.buf);
        let r = self.flush_leaf();
        self.restore(m);
        r
    }
    /// Emit a raw-JSON leaf at the current path.
    pub fn leaf_raw_here(&mut self, f: impl FnOnce(&mut Vec<u8>)) -> io::Result<()> {
        self.buf.clear();
        f(&mut self.buf);
        self.flush_leaf()
    }
    pub fn count(&mut self, name: &str, n: usize) -> io::Result<()> {
        self.leaf_raw(name, |b| {
            let _ = write!(Wr(b), "{{\"u64\":\"{n}\"}}");
        })
    }
    /// Emit a leaf of `kind` for value `v` at the current path.
    pub fn leaf_here(&mut self, kind: K, v: Option<&V>) -> io::Result<()> {
        self.buf.clear();
        let mut buf = std::mem::take(&mut self.buf);
        let r = self.scalar(&mut buf, kind, v);
        self.buf = buf;
        r?;
        self.flush_leaf()
    }
    fn scalar(&self, b: &mut Vec<u8>, kind: K, v: Option<&V>) -> io::Result<()> {
        let bad = || io::Error::other(format!("{}: value does not match {kind:?}", self.path));
        match kind {
            K::Bool | K::Present => {
                let x = match v {
                    None => false,
                    Some(V::Bool(x)) => *x,
                    _ => return Err(bad()),
                };
                b.extend_from_slice(if x { b"true" } else { b"false" });
            }
            K::I64 | K::Enum(_) => {
                let x = match v {
                    None => 0,
                    Some(V::I(x)) => *x,
                    _ => return Err(bad()),
                };
                if let K::Enum(name) = kind {
                    let _ = write!(Wr(b), "{{\"enum\":\"{name}\",\"i64\":\"{x}\"}}");
                } else {
                    let _ = write!(Wr(b), "{{\"i64\":\"{x}\"}}");
                }
            }
            K::U64 => {
                let x = match v {
                    None => 0,
                    Some(V::U(x)) => *x,
                    _ => return Err(bad()),
                };
                let _ = write!(Wr(b), "{{\"u64\":\"{x}\"}}");
            }
            K::F32 => {
                let x = match v {
                    None => 0.0,
                    Some(V::F32(x)) => *x,
                    _ => return Err(bad()),
                };
                let _ = write!(Wr(b), "{{\"f32\":\"{:08x}\"}}", x.to_bits());
            }
            K::F64 => {
                let x = match v {
                    None => 0.0,
                    Some(V::F(x)) => *x,
                    _ => return Err(bad()),
                };
                f64_into(b, x);
            }
            K::Str => {
                let x: &[u8] = match v {
                    None => b"",
                    Some(V::Bytes(x)) => x,
                    _ => return Err(bad()),
                };
                let _ = write!(Wr(b), "{{\"length\":\"{}\",\"bytes_hex\":\"", x.len());
                hex_into(b, x);
                b.extend_from_slice(b"\"}");
            }
            K::Blob => {
                let x: &[u8] = match v {
                    None => b"",
                    Some(V::Bytes(x)) => x,
                    _ => return Err(bad()),
                };
                let _ = write!(
                    Wr(b),
                    "{{\"type\":\"blob\",\"length\":\"{}\",\"sha256\":\"",
                    x.len()
                );
                hex_into(b, &Sha256::digest(x));
                b.extend_from_slice(b"\"}");
            }
            K::V2 | K::V3 | K::V4 | K::Quat | K::Mat => {
                let (tag, n) = match kind {
                    K::V2 => ("vec2", 2),
                    K::V3 => ("vec3", 3),
                    K::V4 => ("vec4", 4),
                    K::Quat => ("quat", 4),
                    _ => ("matrix", 12),
                };
                let zero = [0.0; 12];
                let x: &[f64] = match v {
                    None => &zero[..n],
                    Some(V::Vec(x)) if x.len() == n => x,
                    _ => return Err(bad()),
                };
                vector_into(b, tag, x);
            }
            K::Ref | K::Handle(_) => match v {
                None | Some(V::Ref(None)) => b.extend_from_slice(b"null"),
                Some(V::Ref(Some(i))) => {
                    let key = self.keys.get(*i).ok_or_else(bad)?;
                    b.extend_from_slice(b"{\"ref\":");
                    json_str(b, key);
                    b.push(b'}');
                }
                _ => return Err(bad()),
            },
            K::DomRef => match v {
                None | Some(V::Dom(None)) => b.extend_from_slice(b"null"),
                Some(V::Dom(Some(i))) => {
                    let _ = write!(Wr(b), "{{\"dom_ordinal\":\"{i}\"}}");
                }
                _ => return Err(bad()),
            },
            K::S(_) | K::Opt(_) | K::Arr(..) => return Err(bad()),
        }
        Ok(())
    }
    fn primitive(kind: K) -> bool {
        !matches!(kind, K::S(_) | K::Opt(_) | K::Arr(..))
    }
    /// Emit all fields of schema struct `name` from `obj` (missing fields are zero).
    pub fn emit_struct(&mut self, name: &str, obj: Option<&Obj>) -> io::Result<()> {
        self.depth += 1;
        if self.depth > 256 {
            return Err(io::Error::other("canonical recursion depth"));
        }
        let r = self.emit_struct_inner(name, obj);
        self.depth -= 1;
        r
    }
    fn emit_struct_inner(&mut self, name: &str, obj: Option<&Obj>) -> io::Result<()> {
        match lookup(name)? {
            Body::Fields { named_len, fields } => {
                if let Some(n) = named_len {
                    self.count("length", *n)?;
                }
                for f in fields.iter() {
                    let v = obj.and_then(|o| o.get(f.name));
                    let m = self.push(f.name);
                    self.emit_kind(f.kind, v)?;
                    self.restore(m);
                }
                Ok(())
            }
            Body::List { .. } => {
                let name = crate::canon_schema::STRUCTS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map_or("", |(n, _)| *n);
                self.emit_kind(K::S(name), obj.and_then(|o| o.get("data")))
            }
        }
    }
    /// Emit a value of `kind` at the current path.
    pub fn emit_kind(&mut self, kind: K, v: Option<&V>) -> io::Result<()> {
        match kind {
            K::S(name) => match (lookup(name)?, v) {
                (Body::List { .. }, Some(V::Obj(o))) => self.emit_kind(kind, o.get("data")),
                (Body::List { item, sorter }, _) => match (sorter, v) {
                    (Some(sorter), Some(V::List(items))) => {
                        let sorted = self.sorted(*sorter, items);
                        self.emit_seq(*item, Some(&V::List(sorted)), None)
                    }
                    _ => self.emit_seq(*item, v, None),
                },
                (_, None) => self.emit_struct(name, None),
                (_, Some(V::Obj(o))) => self.emit_struct(name, Some(o)),
                _ => Err(io::Error::other(format!("{}: expected struct", self.path))),
            },
            K::Opt(name) => match v {
                None | Some(V::Null) => self.leaf_here(K::Ref, None),
                other => self.emit_kind(K::S(name), other),
            },
            K::Arr(item, n) => self.emit_seq(*item, v, Some(n)),
            _ => self.leaf_here(kind, v),
        }
    }
    /// canon.c sorts these lists by source identity keys, then byte strings.
    fn sorted(&self, sorter: Sorter, items: &[V]) -> Vec<V> {
        let fields: &[&str] = match sorter {
            Sorter::Connection => &["src", "dst", "src_prop", "dst_prop"],
            Sorter::AnimProp => &["element", "prop_name"],
            Sorter::PropOverride => &["element_id", "prop_name"],
            Sorter::TransformOverride => &["node_id"],
        };
        let key = |v: &V, f: &str| -> Vec<u8> {
            let V::Obj(o) = v else { return Vec::new() };
            match o.get(f) {
                Some(V::Ref(Some(i))) => self
                    .keys
                    .get(*i)
                    .map_or(Vec::new(), |k| k.clone().into_bytes()),
                Some(V::Bytes(b)) => b.clone(),
                _ => Vec::new(),
            }
        };
        let mut out = items.to_vec();
        out.sort_by(|a, b| {
            fields
                .iter()
                .map(|f| key(a, f).cmp(&key(b, f)))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
    fn emit_seq(&mut self, item: K, v: Option<&V>, fixed: Option<usize>) -> io::Result<()> {
        let len = match v {
            None => 0,
            Some(V::List(x)) => x.len(),
            Some(V::F64s(x)) => x.len(),
            Some(V::U32s(x)) => x.len(),
            Some(V::I32s(x)) => x.len(),
            Some(V::Vec2s(x)) => x.len(),
            Some(V::Vec3s(x)) => x.len(),
            Some(V::Vec4s(x)) => x.len(),
            Some(V::Bools(x)) => x.len(),
            Some(V::Vec(x)) => x.len(),
            _ => {
                return Err(io::Error::other(format!(
                    "{}: expected sequence",
                    self.path
                )))
            }
        };
        if let Some(n) = fixed {
            if v.is_some() && len != n {
                return Err(io::Error::other(format!(
                    "{}: fixed array length",
                    self.path
                )));
            }
        }
        let len = fixed.unwrap_or(len);
        self.count("length", len)?;
        if Self::primitive(item) {
            let mut start = 0;
            while start < len {
                let end = (start + 4096).min(len);
                let m = self.push_index(start);
                let mut b = std::mem::take(&mut self.buf);
                b.clear();
                b.push(b'[');
                let mut r = Ok(());
                for i in start..end {
                    if i != start {
                        b.push(b',');
                    }
                    let elem = element(v, i);
                    r = self.scalar(&mut b, item, elem.as_ref());
                    if r.is_err() {
                        break;
                    }
                }
                b.push(b']');
                self.buf = b;
                r?;
                self.flush_leaf()?;
                self.restore(m);
                start = end;
            }
        } else {
            for i in 0..len {
                let m = self.push_index(i);
                let elem = element(v, i);
                self.emit_kind(item, elem.as_ref())?;
                self.restore(m);
            }
        }
        Ok(())
    }
}

/// Owned view of one sequence element (typed vectors are expanded on demand).
#[cfg(feature = "parity")]
fn element(v: Option<&V>, i: usize) -> Option<V> {
    match v? {
        V::List(x) => x.get(i).cloned(),
        V::F64s(x) => x.get(i).map(|&f| V::F(f)),
        V::Vec(x) => x.get(i).map(|&f| V::F(f)),
        V::U32s(x) => x.get(i).map(|&u| V::U(u64::from(u))),
        V::I32s(x) => x.get(i).map(|&u| V::I(i64::from(u))),
        V::Vec2s(x) => x.get(i).map(|a| V::Vec(a.to_vec())),
        V::Vec3s(x) => x.get(i).map(|a| V::Vec(a.to_vec())),
        V::Vec4s(x) => x.get(i).map(|a| V::Vec(a.to_vec())),
        V::Bools(x) => x.get(i).map(|&b| V::Bool(b)),
        _ => None,
    }
}

#[cfg(feature = "parity")]
pub(crate) fn f64_into(b: &mut Vec<u8>, x: f64) {
    let _ = write!(Wr(b), "{{\"f64\":\"{:016x}\"}}", x.to_bits());
}
#[cfg(feature = "parity")]
fn vector_into(b: &mut Vec<u8>, tag: &str, x: &[f64]) {
    let _ = write!(Wr(b), "{{\"{tag}\":[");
    for (i, &c) in x.iter().enumerate() {
        if i > 0 {
            b.push(b',');
        }
        f64_into(b, c);
    }
    b.extend_from_slice(b"]}");
}

/// `fmt::Write` adapter over a byte buffer.
#[cfg(feature = "parity")]
pub(crate) struct Wr<'a>(pub &'a mut Vec<u8>);
#[cfg(feature = "parity")]
impl std::fmt::Write for Wr<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}
#[cfg(feature = "parity")]
pub(crate) fn hex(out: &mut Vec<u8>, bytes: &[u8]) {
    hex_into(out, bytes)
}
