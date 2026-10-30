//! Bounded streaming framing for `fbx-scene-canon/1`.
//!
//! The writer is infrastructure only: no function here converts a scene or
//! supplies the required fields. An unfinished stream has no valid footer.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

/// The `fbx-scene-canon/1` sections, in stream order.
pub const SECTIONS: &[&str] = &[
    "options",
    "globals",
    "dom",
    "elements",
    "connections",
    "graph",
    "meshes",
    "skins",
    "blend_shapes",
    "materials",
    "cameras_lights",
    "curves",
    "advanced",
    "triangles",
    "curve_samples",
    "evaluated",
];

struct HexDigest([u8; 32]);
impl std::fmt::Display for HexDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
struct Line<'a, W> {
    out: &'a mut W,
    file: &'a mut Sha256,
    section: Option<&'a mut Sha256>,
    written: &'a mut usize,
    line: usize,
    max_line: usize,
    max_output: usize,
}
impl<W: Write> Write for Line<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let line = self
            .line
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("canonical line limit"))?;
        let total = self
            .written
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("canonical output limit"))?;
        if line > self.max_line || total > self.max_output {
            return Err(io::Error::other("canonical output limit"));
        }
        self.out.write_all(bytes)?;
        self.file.update(bytes);
        if let Some(section) = self.section.as_deref_mut() {
            section.update(bytes);
        }
        *self.written = total;
        self.line = line;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
/// A stream that only writes its completion marker after all sections close.
pub struct CanonicalWriter<W> {
    out: W,
    file: Sha256,
    section: Sha256,
    index: usize,
    active: bool,
    section_count: u64,
    total_count: u64,
    written: usize,
    max_line: usize,
    max_output: usize,
    poisoned: bool,
}
impl<W: Write> CanonicalWriter<W> {
    /// `receipt` must be the schema's complete receipt record. Callers own its
    /// provenance and must validate the final output before using it as evidence.
    pub fn new(
        out: W,
        receipt: &impl Serialize,
        max_line: usize,
        max_output: usize,
    ) -> io::Result<Self> {
        let mut writer = Self {
            out,
            file: Sha256::new(),
            section: Sha256::new(),
            index: 0,
            active: false,
            section_count: 0,
            total_count: 0,
            written: 0,
            max_line,
            max_output,
            poisoned: false,
        };
        writer.line(false, |line| {
            serde_json::to_writer(&mut *line, receipt)?;
            line.write_all(b"\n")
        })?;
        Ok(writer)
    }
    fn line(
        &mut self,
        in_section: bool,
        f: impl FnOnce(&mut Line<'_, W>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other("canonical stream failed"));
        }
        let section = if in_section {
            Some(&mut self.section)
        } else {
            None
        };
        let mut line = Line {
            out: &mut self.out,
            file: &mut self.file,
            section,
            written: &mut self.written,
            line: 0,
            max_line: self.max_line,
            max_output: self.max_output,
        };
        let result = f(&mut line);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    /// Emit the exact pinned engine import profile option field records
    /// without allocating.
    pub fn import_profile_options(&mut self) -> io::Result<()> {
        let records = crate::import_profile::RECORDS;
        const EXPECTED: [u8; 32] = [
            0x16, 0x91, 0xe8, 0xf6, 0x41, 0x11, 0xec, 0xd4, 0x5b, 0xcb, 0xfd, 0xf7, 0x87, 0xa5,
            0x1b, 0x9f, 0xd1, 0x41, 0xc5, 0x9d, 0x0d, 0x9e, 0x72, 0x6b, 0xa6, 0xcc, 0x1a, 0xfa,
            0xfc, 0x37, 0x00, 0xd9,
        ];
        let digest: [u8; 32] = Sha256::digest(records).into();
        if digest != EXPECTED {
            return Err(io::Error::other("pinned options mismatch"));
        }
        self.begin("options")?;
        for record in records.split_inclusive(|&b| b == b'\n') {
            if record.is_empty() || !record.ends_with(b"\n") {
                self.poisoned = true;
                return Err(io::Error::other("invalid pinned options"));
            }
            let section_count = self
                .section_count
                .checked_add(1)
                .ok_or_else(|| io::Error::other("canonical field count"))?;
            let total_count = self
                .total_count
                .checked_add(1)
                .ok_or_else(|| io::Error::other("canonical field count"))?;
            self.line(true, |line| line.write_all(record))?;
            self.section_count = section_count;
            self.total_count = total_count;
        }
        self.end()
    }
    pub fn begin(&mut self, name: &str) -> io::Result<()> {
        if self.active || SECTIONS.get(self.index) != Some(&name) {
            return Err(io::Error::other("canonical section order"));
        }
        self.line(false, |line| {
            line.write_all(b"{\"kind\":\"begin\",\"name\":")?;
            serde_json::to_writer(&mut *line, name)?;
            line.write_all(b"}\n")
        })?;
        self.section = Sha256::new();
        self.section_count = 0;
        self.active = true;
        Ok(())
    }
    pub fn field(&mut self, path: &str, value: &impl Serialize) -> io::Result<()> {
        if !self.active
            || !path.strip_prefix('/').is_some_and(|p| {
                p.starts_with(SECTIONS[self.index])
                    && p.as_bytes().get(SECTIONS[self.index].len()) == Some(&b'/')
            })
        {
            return Err(io::Error::other("canonical field path"));
        }
        let section_count = self
            .section_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("canonical field count"))?;
        let total_count = self
            .total_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("canonical field count"))?;
        self.line(true, |line| {
            line.write_all(b"{\"kind\":\"field\",\"path\":")?;
            serde_json::to_writer(&mut *line, path)?;
            line.write_all(b",\"value\":")?;
            serde_json::to_writer(&mut *line, value)?;
            line.write_all(b"}\n")
        })?;
        self.section_count = section_count;
        self.total_count = total_count;
        Ok(())
    }
    /// Emit a field whose value is already canonical JSON bytes.
    pub fn field_raw(&mut self, path: &str, value: &[u8]) -> io::Result<()> {
        if !self.active {
            return Err(io::Error::other("canonical field path"));
        }
        let section_count = self
            .section_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("canonical field count"))?;
        let total_count = self
            .total_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("canonical field count"))?;
        self.line(true, |line| {
            line.write_all(b"{\"kind\":\"field\",\"path\":")?;
            serde_json::to_writer(&mut *line, path)?;
            line.write_all(b",\"value\":")?;
            line.write_all(value)?;
            line.write_all(b"}\n")
        })?;
        self.section_count = section_count;
        self.total_count = total_count;
        Ok(())
    }
    pub fn end(&mut self) -> io::Result<()> {
        if !self.active {
            return Err(io::Error::other("no active canonical section"));
        }
        let name = SECTIONS[self.index];
        let count = self.section_count;
        let hash = HexDigest(self.section.clone().finalize().into());
        self.line(false,|line| writeln!(line,"{{\"kind\":\"section\",\"name\":\"{name}\",\"count\":\"{count}\",\"sha256\":\"{hash}\"}}"))?;
        self.active = false;
        self.index += 1;
        Ok(())
    }
    pub fn finish(mut self) -> io::Result<W> {
        if self.poisoned || self.active || self.index != SECTIONS.len() {
            return Err(io::Error::other("incomplete canonical stream"));
        }
        let hash = HexDigest(self.file.clone().finalize().into());
        let count = self.total_count;
        self.line(false,|line| writeln!(line,"{{\"kind\":\"footer\",\"records\":\"{count}\",\"sha256\":\"{hash}\",\"exit_status\":\"0\"}}"))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Serialize schema value maps in the field order used by the oracle writer (reference/canon.c).
/// In particular, the pinned options section hashes its exact field bytes.
pub struct CanonicalValue<'a>(pub &'a serde_json::Value);
impl Serialize for CanonicalValue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self.0 {
            serde_json::Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for v in values {
                    seq.serialize_element(&CanonicalValue(v))?;
                }
                seq.end()
            }
            serde_json::Value::Object(map) => {
                let mut out = serializer.serialize_map(Some(map.len()))?;
                if map.len() == 2 && map.contains_key("length") && map.contains_key("bytes_hex") {
                    for key in ["length", "bytes_hex"] {
                        out.serialize_entry(key, &CanonicalValue(&map[key]))?;
                    }
                } else if map.len() == 3
                    && map.contains_key("type")
                    && map.contains_key("length")
                    && map.contains_key("sha256")
                {
                    for key in ["type", "length", "sha256"] {
                        out.serialize_entry(key, &CanonicalValue(&map[key]))?;
                    }
                } else {
                    for (k, v) in map {
                        out.serialize_entry(k, &CanonicalValue(v))?;
                    }
                }
                out.end()
            }
            serde_json::Value::Number(_) => Err(serde::ser::Error::custom(
                "canonical values cannot contain JSON numbers",
            )),
            other => other.serialize(serializer),
        }
    }
}

/// Bounds for an experimental source-coordinate triangles section.
#[derive(Debug, Clone)]
pub struct TriangleDumpLimits {
    pub max_allocation: usize,
    pub max_face_vertices: usize,
    pub max_work: usize,
}
impl Default for TriangleDumpLimits {
    fn default() -> Self {
        Self {
            max_allocation: 256 << 20,
            max_face_vertices: 1 << 20,
            max_work: 1 << 30,
        }
    }
}
struct Count(usize);
impl Serialize for Count {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Decimal(usize);
        impl Serialize for Decimal {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&self.0)
            }
        }
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("u64", &Decimal(self.0))?;
        map.end()
    }
}

struct Signed(i64);
impl Serialize for Signed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Decimal(i64);
        impl Serialize for Decimal {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&self.0)
            }
        }
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("i64", &Decimal(self.0))?;
        map.end()
    }
}
struct EnumValue(&'static str, i64);
impl Serialize for EnumValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Decimal(i64);
        impl Serialize for Decimal {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&self.0)
            }
        }
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("enum", self.0)?;
        map.serialize_entry("i64", &Decimal(self.1))?;
        map.end()
    }
}
struct ByteHex<'a>(&'a [u8]);
impl std::fmt::Display for ByteHex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for ByteHex<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
struct ByteString<'a>(&'a [u8]);
impl Serialize for ByteString<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Decimal(usize);
        impl Serialize for Decimal {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&self.0)
            }
        }
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("length", &Decimal(self.0.len()))?;
        map.serialize_entry("bytes_hex", &ByteHex(self.0))?;
        map.end()
    }
}
struct Blob<'a>(&'a [u8]);
impl Serialize for Blob<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Decimal(usize);
        impl Serialize for Decimal {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&self.0)
            }
        }
        struct DigestText([u8; 32]);
        impl Serialize for DigestText {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(&HexDigest(self.0))
            }
        }
        let mut map = s.serialize_map(Some(3))?;
        map.serialize_entry("type", "blob")?;
        map.serialize_entry("length", &Decimal(self.0.len()))?;
        map.serialize_entry("sha256", &DigestText(Sha256::digest(self.0).into()))?;
        map.end()
    }
}
struct Real(f64);
impl Serialize for Real {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        struct Bits(u64);
        impl std::fmt::Display for Bits {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{:016x}", self.0)
            }
        }
        impl Serialize for Bits {
            fn serialize<T: serde::Serializer>(&self, s: T) -> Result<T::Ok, T::Error> {
                s.collect_str(self)
            }
        }
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("f64", &Bits(self.0.to_bits()))?;
        map.end()
    }
}
struct RealFour([f64; 4]);
impl Serialize for RealFour {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(4))?;
        for v in self.0 {
            seq.serialize_element(&Real(v))?;
        }
        seq.end()
    }
}

/// Emit only the source global-settings property list inside an already-open
/// `globals` section. This deliberately does not complete that section or a
/// scene stream. Non-source/synthetic global properties remain unimplemented.
pub fn write_source_global_properties<W: Write>(
    writer: &mut CanonicalWriter<W>,
    properties: &[crate::scene_property::AuthoredProperty<'_>],
    max_path_allocation: usize,
) -> io::Result<()> {
    use std::fmt::Write as _;
    let result = (|| {
        if max_path_allocation < 256 {
            return Err(io::Error::other("canonical path allocation limit"));
        }
        let mut bytes = Vec::new();
        crate::allocation::reserve(&mut bytes, 256, &mut 0, max_path_allocation)
            .map_err(|_| io::Error::other("canonical path allocation limit or failure"))?;
        let mut path = String::from_utf8(bytes).unwrap();
        let base = "/globals/settings/props";
        writer.field(
            "/globals/settings/props/props/length",
            &Count(properties.len()),
        )?;
        for (i, prop) in properties.iter().enumerate() {
            macro_rules! field {
                ($name:literal, $value:expr) => {{
                    path.clear();
                    write!(&mut path, "{base}/props/{i}/{}", $name)
                        .map_err(|_| io::Error::other("path format"))?;
                    writer.field(&path, &$value)?;
                }};
            }
            field!("name", ByteString(prop.name));
            field!("type", EnumValue("ufbx_prop_type", prop.kind as i64));
            field!("flags", EnumValue("ufbx_prop_flags", i64::from(prop.flags)));
            field!("value_str", ByteString(prop.value_str));
            field!("value_blob", Blob(prop.value_blob));
            field!("value_int", Signed(prop.value_int));
            field!("value_real_arr/length", Count(4));
            field!("value_real_arr/0", RealFour(prop.value_real));
        }
        writer.field("/globals/settings/props/num_animated", &Count(0))?;
        writer.field("/globals/settings/props/defaults", &Option::<u8>::None)?;
        Ok(())
    })();
    if result.is_err() {
        writer.poisoned = true;
    }
    result
}
struct TriangleChunk<'a>(&'a [usize]);
impl Serialize for TriangleChunk<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for &v in self.0 {
            seq.serialize_element(&Count(v))?;
        }
        seq.end()
    }
}
/// Emit the oracle's triangle field shape from authored geometry. It does not
/// apply the import profile's geometry conversion or winding, so it is not a
/// complete scene dump.
/// An error poisons the stream and prevents a footer from being emitted.
pub fn write_source_triangles<W: Write>(
    writer: &mut CanonicalWriter<W>,
    index: &crate::scene::SceneIndex<'_>,
    meshes: &crate::scene_mesh::Meshes<'_>,
    limits: &TriangleDumpLimits,
) -> io::Result<()> {
    use std::fmt::Write as _;
    let result = (|| -> io::Result<()> {
        let fail = || io::Error::other("triangle dump limit or malformed scene");
        let base = meshes
            .meshes
            .len()
            .checked_mul(std::mem::size_of::<(i64, usize, usize)>())
            .and_then(|v| v.checked_add(160))
            .ok_or_else(fail)?;
        if base > limits.max_allocation {
            return Err(fail());
        }
        let mut retained = 0;
        let mut order = Vec::new();
        crate::allocation::reserve(
            &mut order,
            meshes.meshes.len(),
            &mut retained,
            limits.max_allocation,
        )
        .map_err(|_| fail())?;
        for (i, m) in meshes.meshes.iter().enumerate() {
            let obj = index.objects.get(m.object).ok_or_else(fail)?;
            order.push((obj.id, m.node, i));
        }
        order.sort_unstable();
        let mut bytes = Vec::new();
        crate::allocation::reserve(&mut bytes, 160, &mut retained, limits.max_allocation)
            .map_err(|_| fail())?;
        let mut path = String::from_utf8(bytes).unwrap();
        writer.begin("triangles")?;
        writer.field("/triangles/length", &Count(order.len()))?;
        let mut work_left = limits.max_work;
        for (id, node, mi) in order {
            let mesh = meshes.meshes.get(mi).ok_or_else(fail)?;
            let topology_work = mesh
                .vertices
                .ok_or_else(fail)?
                .len()
                .checked_add(mesh.polygon_indices.ok_or_else(fail)?.len())
                .ok_or_else(fail)?;
            work_left = work_left.checked_sub(topology_work).ok_or_else(fail)?;
            let topo = mesh.validate_topology().map_err(|_| fail())?;
            path.clear();
            write!(
                &mut path,
                "/triangles/id:{id}:ordinal:{}/faces",
                node.checked_add(1).ok_or_else(fail)?
            )
            .map_err(|_| fail())?;
            writer.field(&path, &Count(topo.faces))?;
            for (face, range) in mesh.faces().map_err(|_| fail())?.enumerate() {
                let range = range.map_err(|_| fail())?;
                let n = range.len();
                let cost = if n > 4 {
                    n.checked_mul(n)
                        .and_then(|v| v.checked_mul(n))
                        .and_then(|v| v.checked_mul(4))
                        .and_then(|v| v.checked_add(n.saturating_mul(6)))
                        .ok_or_else(fail)?
                } else {
                    n
                };
                work_left = work_left.checked_sub(cost).ok_or_else(fail)?;
                let needed = n.saturating_sub(2).checked_mul(3).ok_or_else(fail)?;
                let output_bytes = needed
                    .checked_mul(std::mem::size_of::<usize>())
                    .ok_or_else(fail)?;
                if retained.checked_add(output_bytes).ok_or_else(fail)? > limits.max_allocation {
                    return Err(fail());
                }
                let mut triangles = Vec::new();
                let mut used = retained;
                crate::allocation::reserve(
                    &mut triangles,
                    needed,
                    &mut used,
                    limits.max_allocation,
                )
                .map_err(|_| fail())?;
                triangles.resize(needed, 0);
                let available = limits.max_allocation.checked_sub(used).ok_or_else(fail)?;
                let tri_limits = crate::scene_mesh::TriangulationLimits {
                    max_face_vertices: limits.max_face_vertices,
                    max_allocation: available,
                    max_work: cost,
                };
                mesh.triangulate_with_limits(range.start, n, &mut triangles, &tri_limits)
                    .map_err(|_| fail())?;
                path.clear();
                write!(
                    &mut path,
                    "/triangles/id:{id}:ordinal:{}/{face}/length",
                    node + 1
                )
                .map_err(|_| fail())?;
                writer.field(&path, &Count(needed))?;
                for (chunk_index, chunk) in triangles.chunks(4096).enumerate() {
                    path.clear();
                    write!(
                        &mut path,
                        "/triangles/id:{id}:ordinal:{}/{face}/{}",
                        node + 1,
                        chunk_index * 4096
                    )
                    .map_err(|_| fail())?;
                    writer.field(&path, &TriangleChunk(chunk))?;
                }
            }
        }
        writer.end()
    })();
    if result.is_err() {
        writer.poisoned = true;
    }
    result
}
