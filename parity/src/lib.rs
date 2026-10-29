//! Parity tooling for `fbx-rs` (unpublished): the `fbx-scene-canon/1`
//! stream validator, the oracle and scene comparators, and the source binding
//! of Rust scene dumps. The canonical dump itself is
//! `fbx_rs::canon_scene::dump` behind the unstable `parity` feature.
#![forbid(unsafe_code)]

pub use fbx_rs::canonical_writer::SECTIONS;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const MAX_LINE: usize = 4 * 1024 * 1024;
const PINS: &str = include_str!("../../reference/pins.json");
const HISTORICAL_OPTIONS: &str = include_str!("../../reference/options-historical-raw.json");
const CONVERTED_OPTIONS: &str = include_str!("../../reference/options-engine-converted.json");

fn expected_options(profile: &str) -> Result<Value> {
    serde_json::from_str(if profile == "historical-raw" {
        HISTORICAL_OPTIONS
    } else {
        CONVERTED_OPTIONS
    })
    .map_err(|e| e.to_string())
}

type Result<T> = std::result::Result<T, String>;

/// Reject duplicate JSON keys instead of silently replacing their first value.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique keys and no numeric JSON literals")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Bool(v)))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(v.to_owned())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::String(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(v)) = a.next_element()? {
                    values.push(v);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) {
                        return Err(de::Error::custom(format!("duplicate key {k}")));
                    }
                    let Unique(v) = a.next_value()?;
                    values.insert(k, v);
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        d.deserialize_any(UniqueVisitor)
    }
}

fn parse(line: &[u8]) -> Result<Value> {
    serde_json::from_slice::<Unique>(line)
        .map(|v| v.0)
        .map_err(|e| format!("invalid canonical JSON: {e}"))
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing/string field {key}"))
}
fn decimal(s: &str) -> bool {
    s == "0" || (!s.starts_with('0') && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
fn count(v: &Value, key: &str) -> Result<u64> {
    let s = string(v, key)?;
    if !decimal(s) {
        return Err(format!("invalid decimal {key}={s}"));
    }
    s.parse().map_err(|_| format!("count overflow: {key}"))
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash_string(h: Sha256) -> String {
    format!("{:x}", h.finalize())
}
fn read_record(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> Result<Option<Value>> {
    buffer.clear();
    // take() bounds growth even for a malformed line without a terminating LF.
    use std::io::Read;
    let n = reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', buffer)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > MAX_LINE || !buffer.ends_with(b"\n") || buffer.ends_with(b"\r\n") {
        return Err("line too long, CRLF, or truncated line".into());
    }
    parse(buffer).map(Some)
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub header: Value,
    pub records: u64,
    pub sections: Vec<(String, u64)>,
}

fn validate_receipt(v: &Value) -> Result<()> {
    for (key, expected) in [
        ("schema", "fbx-scene-canon/1"),
        ("exit_status", "0"),
        ("real", "f64"),
        ("external_open", "denied"),
    ] {
        if string(v, key)? != expected {
            return Err(format!("receipt/{key}: expected {expected}"));
        }
    }
    let pins: Value = serde_json::from_str(PINS).map_err(|e| e.to_string())?;
    let profile = string(v, "profile")?;
    let pin = pins
        .get(profile)
        .ok_or_else(|| format!("unknown oracle profile {profile}"))?;
    for (field, source) in [("source_sha256", "ufbx.c"), ("header_sha256", "ufbx.h")] {
        if string(v, field)? != string(pin, source)? {
            return Err(format!("receipt/{field}: pin mismatch for {profile}"));
        }
    }
    for field in ["input_sha256", "coverage_sha256", "dumper_sha256"] {
        if !hex(string(v, field)?, 64) {
            return Err(format!("invalid receipt/{field}"));
        }
    }
    let coverage = if profile == "historical-raw" {
        include_bytes!("../../reference/coverage-historical-raw.json").as_slice()
    } else {
        include_bytes!("../../reference/coverage-engine-converted.json").as_slice()
    };
    if string(v, "coverage_sha256")? != format!("{:x}", Sha256::digest(coverage)) {
        return Err("receipt/coverage_sha256: field schema mismatch".into());
    }
    count(v, "input_length")?;
    Ok(())
}

/// Closed set of evaluated paths and component counts. No dump-supplied rule
/// can turn an authored value, a count, a key time or topology into a tolerance.
fn evaluated_width(path: &str, quaternion: bool) -> Option<usize> {
    let p: Vec<_> = path.split('/').collect();
    if !quaternion && p.len() == 5 && p[1] == "curve_samples" && decimal(p[3]) && p[4] == "value" {
        return Some(1);
    }
    if p.len() < 6 || p[1] != "evaluated" || !decimal(p[3]) {
        return None;
    }
    match p[4] {
        "nodes" if p.len() == 8 && ["local", "geometry"].contains(&p[6]) => {
            match (p[7], quaternion) {
                ("rotation", true) => Some(4),
                ("translation" | "scale", false) => Some(3),
                _ => None,
            }
        }
        "nodes" if p.len() == 7 && !quaternion => match p[6] {
            "node_to_parent" | "node_to_world" | "geometry_to_node" | "geometry_to_world" => {
                Some(12)
            }
            _ => None,
        },
        "skins" if p.len() == 7 && p[6] == "geometry_to_world" && !quaternion => Some(12),
        "morphs" if p.len() == 7 && p[6] == "weight" && !quaternion => Some(1),
        "layers"
            if p.len() == 10
                && decimal(p[5])
                && decimal(p[6])
                && ["layer", "composed"].contains(&p[7])
                && p[8] == "components"
                && !quaternion =>
        {
            None
        }
        // /evaluated/stack/sample/layers/layer/property/composition/components
        "layers"
            if p.len() == 9
                && decimal(p[5])
                && decimal(p[6])
                && ["layer", "composed"].contains(&p[7])
                && p[8] == "components"
                && !quaternion =>
        {
            Some(4)
        }
        _ => None,
    }
}
fn evaluated_values<'a>(path: &str, value: &'a Value) -> Result<Option<(&'a Vec<Value>, bool)>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let key = if obj.contains_key("eval_f64") {
        "eval_f64"
    } else if obj.contains_key("eval_quat") {
        "eval_quat"
    } else {
        return Ok(None);
    };
    let quaternion = key == "eval_quat";
    let width =
        evaluated_width(path, quaternion).ok_or_else(|| format!("{path}: undeclared tolerance"))?;
    let values = obj[key]
        .as_array()
        .ok_or_else(|| format!("{path}: evaluated value is not an array"))?;
    if obj.len() != 1 || values.len() != width {
        return Err(format!("{path}: evaluated component count mismatch"));
    }
    for v in values {
        let x = float(v, 64)?;
        if !x.is_finite() {
            return Err(format!("{path}: nonfinite evaluated value"));
        }
    }
    if quaternion
        && values
            .iter()
            .map(|v| float(v, 64).unwrap_or(0.0).powi(2))
            .sum::<f64>()
            == 0.0
    {
        return Err(format!("{path}: zero quaternion"));
    }
    Ok(Some((values, quaternion)))
}
fn float(v: &Value, bits: usize) -> Result<f64> {
    let s = v.as_str().ok_or("float bits must be a string")?;
    if !hex(s, bits / 4) {
        return Err(format!("invalid f{bits} bits: {s}"));
    }
    let word = u64::from_str_radix(s, 16).map_err(|e| e.to_string())?;
    Ok(if bits == 32 {
        f32::from_bits(word as u32) as f64
    } else {
        f64::from_bits(word)
    })
}
fn validate_value(path: &str, v: &Value) -> Result<()> {
    if evaluated_values(path, v)?.is_some() {
        return Ok(());
    }
    match v {
        Value::Object(obj) => {
            for (k, value) in obj {
                match k.as_str() {
                    "f32" | "f64" => {
                        float(value, if k == "f32" { 32 } else { 64 })?;
                    }
                    "u64" => {
                        count(v, k)?;
                    }
                    "length" => {
                        count(v, k)?;
                    }
                    "sha256" => {
                        if !hex(string(v, k)?, 64) {
                            return Err(format!("{path}: invalid blob SHA-256"));
                        }
                    }
                    "i64" => {
                        let s = string(v, k)?;
                        let number = s
                            .parse::<i64>()
                            .map_err(|_| format!("{path}: invalid i64"))?;
                        if number.to_string() != s {
                            return Err(format!("{path}: noncanonical i64"));
                        }
                    }
                    "bytes_hex" => {
                        let s = string(v, k)?;
                        if s.len() % 2 != 0 || !hex(s, s.len()) {
                            return Err(format!("{path}: invalid bytes_hex"));
                        }
                        if obj.contains_key("length") && count(v, "length")? != (s.len() / 2) as u64
                        {
                            return Err(format!("{path}: string byte count mismatch"));
                        }
                    }
                    "eval_f64" | "eval_quat" => {
                        return Err(format!("{path}: nested tolerance tag"))
                    }
                    _ => validate_value(path, value)?,
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_value(path, value)?;
            }
        }
        _ => (),
    }
    Ok(())
}

fn collect_references(value: &Value, references: &mut HashSet<String>) -> Result<()> {
    match value {
        Value::Object(object) => {
            if object.contains_key("ref") {
                let key = string(value, "ref")?;
                if key.is_empty() || object.len() != 1 {
                    return Err("malformed source reference".into());
                }
                references.insert(key.to_owned());
            } else {
                for child in object.values() {
                    collect_references(child, references)?;
                }
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_references(child, references)?;
            }
        }
        _ => (),
    }
    Ok(())
}

/// Validate the whole stream before trusting any equality result or difference.
pub fn validate(reader: &mut impl BufRead) -> Result<Receipt> {
    let mut buffer = Vec::new();
    let first = read_record(reader, &mut buffer)?.ok_or("missing receipt")?;
    if string(&first, "kind")? != "receipt" {
        return Err("first record must be receipt".into());
    }
    validate_receipt(&first)?;
    let mut file_hash = Sha256::new();
    file_hash.update(&buffer);
    let mut section_hash = Sha256::new();
    let mut active: Option<String> = None;
    let mut sections = Vec::new();
    let mut records = 0u64;
    let mut section_count = 0u64;
    let mut elements = HashSet::new();
    let mut references = HashSet::new();
    let mut element_count = None;
    loop {
        let v = read_record(reader, &mut buffer)?.ok_or("truncated dump: missing footer")?;
        match string(&v, "kind")? {
            "begin" => {
                let name = string(&v, "name")?;
                if active.is_some() || SECTIONS.get(sections.len()) != Some(&name) {
                    return Err(format!("section order/duplicate error: {name}"));
                }
                active = Some(name.to_owned());
                section_hash = Sha256::new();
                section_count = 0;
            }
            "field" => {
                let name = active.as_deref().ok_or("field outside section")?;
                let path = string(&v, "path")?;
                if !path.starts_with(&format!("/{name}/")) {
                    return Err(format!("path outside section: {path}"));
                }
                let value = v.get("value").ok_or("missing field value")?;
                validate_value(path, value)?;
                collect_references(value, &mut references)?;
                if path == "/elements/length" {
                    element_count = Some(count(value, "u64")?);
                } else if let Some(index) = path.strip_prefix("/elements/") {
                    if decimal(index) {
                        let key = string(value, "ref")?;
                        if !elements.insert(key.to_owned()) {
                            return Err(format!("{path}: duplicate source identity {key}"));
                        }
                    }
                }
                section_hash.update(&buffer);
                section_count += 1;
                records += 1;
            }
            "section" => {
                let name = string(&v, "name")?;
                if active.as_deref() != Some(name) {
                    return Err("section terminator mismatch".into());
                }
                if count(&v, "count")? != section_count {
                    return Err(format!("{name}: section count mismatch"));
                }
                if string(&v, "sha256")? != hash_string(section_hash.clone()) {
                    return Err(format!("{name}: section checksum mismatch"));
                }
                if name == "options" {
                    let options = expected_options(string(&first, "profile")?)?;
                    if v["sha256"] != options["section_sha256"] {
                        return Err("options: pinned load options mismatch".into());
                    }
                }
                sections.push((name.to_owned(), section_count));
                active = None;
            }
            "footer" => {
                if active.is_some() || sections.len() != SECTIONS.len() {
                    return Err("missing section(s)".into());
                }
                if count(&v, "records")? != records {
                    return Err("footer record count mismatch".into());
                }
                if string(&v, "sha256")? != hash_string(file_hash) {
                    return Err("file checksum mismatch".into());
                }
                if string(&v, "exit_status")? != "0" {
                    return Err("nonzero oracle exit status".into());
                }
                if read_record(reader, &mut buffer)?.is_some() {
                    return Err("trailing records after footer".into());
                }
                if element_count != Some(elements.len() as u64) {
                    return Err("/elements/length: identity count mismatch".into());
                }
                if let Some(missing) = references.difference(&elements).min() {
                    return Err(format!("dangling source reference {missing}"));
                }
                return Ok(Receipt {
                    header: first,
                    records,
                    sections,
                });
            }
            other => return Err(format!("unknown record kind: {other}")),
        }
        file_hash.update(&buffer);
    }
}

fn mismatch(path: &str, a: &Value, b: &Value) -> String {
    format!("{path}: expected {a}, actual {b}")
}
fn angular(a: &[f64], b: &[f64]) -> f64 {
    let norm_a = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    let a: Vec<_> = a.iter().map(|v| v / norm_a).collect();
    let mut b: Vec<_> = b.iter().map(|v| v / norm_b).collect();
    if a.iter().zip(&b).map(|(x, y)| x * y).sum::<f64>() < 0.0 {
        b.iter_mut().for_each(|v| *v = -*v);
    }
    let minus = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt();
    let plus = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x + y).powi(2))
        .sum::<f64>()
        .sqrt();
    4.0 * minus.atan2(plus)
}
fn difference(path: &str, a: &Value, b: &Value) -> Result<()> {
    if let Some((av, quaternion)) = evaluated_values(path, a)? {
        let Some((bv, bq)) = evaluated_values(path, b)? else {
            return Err(mismatch(path, a, b));
        };
        if quaternion != bq || av.len() != bv.len() {
            return Err(mismatch(path, a, b));
        }
        let av: Vec<_> = av.iter().map(|v| float(v, 64)).collect::<Result<_>>()?;
        let bv: Vec<_> = bv.iter().map(|v| float(v, 64)).collect::<Result<_>>()?;
        let equal = if quaternion {
            angular(&av, &bv) <= 1e-7
        } else {
            av.iter()
                .zip(&bv)
                .all(|(x, y)| (x - y).abs() <= 1e-9 + 1e-8 * x.abs().max(y.abs()))
        };
        return if equal {
            Ok(())
        } else {
            Err(mismatch(path, a, b))
        };
    }
    if a == b {
        return Ok(());
    }
    if let (Some(x), Some(y)) = (
        a.get("bytes_hex").and_then(Value::as_str),
        b.get("bytes_hex").and_then(Value::as_str),
    ) {
        if x != y {
            let offset = x
                .as_bytes()
                .chunks(2)
                .zip(y.as_bytes().chunks(2))
                .position(|(a, b)| a != b)
                .unwrap_or(x.len().min(y.len()) / 2);
            let expected = x.get(offset * 2..offset * 2 + 2).unwrap_or("EOF");
            let actual = y.get(offset * 2..offset * 2 + 2).unwrap_or("EOF");
            return Err(format!("{path}/bytes_hex: first byte offset {offset} within string/chunk, expected {expected}, actual {actual}"));
        }
    }
    match (a, b) {
        (Value::Array(aa), Value::Array(bb)) if aa.len() == bb.len() => {
            for (i, (x, y)) in aa.iter().zip(bb).enumerate() {
                difference(&format!("{path}[{i}]"), x, y)?;
            }
        }
        (Value::Object(aa), Value::Object(bb)) if aa.keys().eq(bb.keys()) => {
            for (key, x) in aa {
                difference(&format!("{path}/{key}"), x, &bb[key])?;
            }
        }
        _ => return Err(mismatch(path, a, b)),
    }
    Ok(())
}

/// Source binding for Rust scene dumps, including parser, converter, framing,
/// schema manifests and dependency lockfile.
pub fn rust_scene_source_sha256() -> String {
    let files: &[(&str, &[u8])] = &[
        ("Cargo.toml", include_bytes!("../../Cargo.toml")),
        ("Cargo.lock", include_bytes!("../../Cargo.lock")),
        ("parity/Cargo.toml", include_bytes!("../Cargo.toml")),
        (
            "reference/pins.json",
            include_bytes!("../../reference/pins.json"),
        ),
        (
            "reference/coverage-engine-converted.json",
            include_bytes!("../../reference/coverage-engine-converted.json"),
        ),
        (
            "reference/options-engine-converted.json",
            include_bytes!("../../reference/options-engine-converted.json"),
        ),
        (
            "reference/canon.c",
            include_bytes!("../../reference/canon.c"),
        ),
        (
            "src/import_profile.rs",
            include_bytes!("../../src/import_profile.rs"),
        ),
        ("parity/src/main.rs", include_bytes!("main.rs")),
        ("parity/src/lib.rs", include_bytes!("lib.rs")),
        (
            "parity/src/check_public.rs",
            include_bytes!("check_public.rs"),
        ),
        ("src/lib.rs", include_bytes!("../../src/lib.rs")),
        ("src/syntax.rs", include_bytes!("../../src/syntax.rs")),
        (
            "src/syntax/ascii.rs",
            include_bytes!("../../src/syntax/ascii.rs"),
        ),
        ("src/scene.rs", include_bytes!("../../src/scene.rs")),
        (
            "src/scene_draft.rs",
            include_bytes!("../../src/scene_draft.rs"),
        ),
        (
            "src/scene_transform.rs",
            include_bytes!("../../src/scene_transform.rs"),
        ),
        (
            "src/scene_mesh.rs",
            include_bytes!("../../src/scene_mesh.rs"),
        ),
        (
            "src/scene_node.rs",
            include_bytes!("../../src/scene_node.rs"),
        ),
        (
            "src/scene_geometry.rs",
            include_bytes!("../../src/scene_geometry.rs"),
        ),
        (
            "src/scene_material.rs",
            include_bytes!("../../src/scene_material.rs"),
        ),
        (
            "src/scene_surface.rs",
            include_bytes!("../../src/scene_surface.rs"),
        ),
        (
            "src/scene_property.rs",
            include_bytes!("../../src/scene_property.rs"),
        ),
        (
            "src/scene_attribute.rs",
            include_bytes!("../../src/scene_attribute.rs"),
        ),
        ("src/deform.rs", include_bytes!("../../src/deform.rs")),
        (
            "src/scene_deform.rs",
            include_bytes!("../../src/scene_deform.rs"),
        ),
        ("src/api.rs", include_bytes!("../../src/api.rs")),
        (
            "src/canonical_writer.rs",
            include_bytes!("../../src/canonical_writer.rs"),
        ),
        ("src/canon.rs", include_bytes!("../../src/canon.rs")),
        ("src/canon_dom.rs", include_bytes!("../../src/canon_dom.rs")),
        (
            "src/canon_schema.rs",
            include_bytes!("../../src/canon_schema.rs"),
        ),
        (
            "src/canon_scene.rs",
            include_bytes!("../../src/canon_scene.rs"),
        ),
    ];
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

fn validate_scene_receipts(oracle: &Value, rust: &Value) -> Result<()> {
    if string(oracle, "profile")? != "engine-converted"
        || string(rust, "profile")? != "engine-converted"
    {
        return Err("scene comparison requires the pinned engine-converted profile".into());
    }
    let reference_hash = format!(
        "{:x}",
        Sha256::digest(include_bytes!("../../reference/canon.c"))
    );
    if string(oracle, "dumper_sha256")? != reference_hash {
        return Err("expected oracle dumper source mismatch".into());
    }
    if string(rust, "producer")? != "rust-scene-v1"
        || string(rust, "dumper_sha256")? != rust_scene_source_sha256()
    {
        return Err("Rust scene producer source mismatch".into());
    }
    Ok(())
}

/// Compare a complete Rust scene stream to the pinned engine-converted oracle.
/// Both complete streams are validated; only their independently bound producer
/// source hashes may differ. The existing oracle comparator remains unchanged.
pub fn compare_scene_files(a: &Path, b: &Path) -> Result<Receipt> {
    compare_files_mode(a, b, false, true)
}

/// Compare two complete independently validated oracle streams; partial
/// streams fail.
pub fn compare_files(a: &Path, b: &Path, allow_profile_mixing: bool) -> Result<Receipt> {
    compare_files_mode(a, b, allow_profile_mixing, false)
}
fn compare_files_mode(
    a: &Path,
    b: &Path,
    allow_profile_mixing: bool,
    rust_scene: bool,
) -> Result<Receipt> {
    let open = |p: &Path| {
        File::open(p)
            .map(BufReader::new)
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let ar = validate(&mut open(a)?).map_err(|e| format!("expected dump invalid: {e}"))?;
    let br = validate(&mut open(b)?).map_err(|e| format!("actual dump invalid: {e}"))?;
    let ap = string(&ar.header, "profile")?;
    let bp = string(&br.header, "profile")?;
    if rust_scene {
        validate_scene_receipts(&ar.header, &br.header)?;
    }
    if ap != bp && !allow_profile_mixing {
        return Err(format!(
            "profile mixing refused: {ap} vs {bp}; explicit --allow-profile-mixing required"
        ));
    }
    for key in ["input_sha256", "input_length", "schema"] {
        difference(&format!("/receipt/{key}"), &ar.header[key], &br.header[key])?;
    }
    if !allow_profile_mixing || ap == bp {
        for key in [
            "source_sha256",
            "header_sha256",
            "coverage_sha256",
            "dumper_sha256",
        ] {
            if rust_scene && key == "dumper_sha256" {
                continue;
            }
            difference(&format!("/receipt/{key}"), &ar.header[key], &br.header[key])?;
        }
    }
    let context = format!("input={} profile={ap}", string(&ar.header, "input_sha256")?);
    let mut a = open(a)?;
    let mut b = open(b)?;
    let (mut ab, mut bb) = (Vec::new(), Vec::new());
    read_record(&mut a, &mut ab)?;
    read_record(&mut b, &mut bb)?;
    for (section, _) in &ar.sections {
        // Consume section delimiters, then walk each field in schema order.
        read_record(&mut a, &mut ab)?;
        read_record(&mut b, &mut bb)?;
        let skip = allow_profile_mixing && ap != bp && section == "options";
        if skip {
            for (reader, buffer) in [(&mut a, &mut ab), (&mut b, &mut bb)] {
                loop {
                    let v = read_record(reader, buffer)?.ok_or("unexpected EOF")?;
                    if string(&v, "kind")? == "section" {
                        break;
                    }
                }
            }
            continue;
        }
        loop {
            let x = read_record(&mut a, &mut ab)?.ok_or("unexpected EOF")?;
            let y = read_record(&mut b, &mut bb)?.ok_or("unexpected EOF")?;
            let xkind = string(&x, "kind")?;
            let ykind = string(&y, "kind")?;
            if xkind == "section" || ykind == "section" {
                if xkind != ykind {
                    return Err(format!("{context} /{section}: record count differs"));
                }
                difference(
                    &format!("/{section}/record_count"),
                    &x["count"],
                    &y["count"],
                )
                .map_err(|e| format!("{context} {e}"))?;
                break;
            }
            let path = string(&x, "path")?;
            if string(&y, "path")? != path {
                return Err(format!(
                    "{context} {path}: field order/path differs, actual {}",
                    y["path"]
                ));
            }
            difference(path, &x["value"], &y["value"]).map_err(|e| format!("{context} {e}"))?;
        }
    }
    Ok(ar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn f(x: f64) -> Value {
        json!({"f64": format!("{:016x}", x.to_bits())})
    }
    fn e(x: &[f64]) -> Value {
        json!({"eval_f64": x.iter().map(|v| format!("{:016x}", v.to_bits())).collect::<Vec<_>>()})
    }

    #[test]
    fn scene_comparison_requires_distinct_bound_producers() {
        let oracle = json!({"profile":"engine-converted","dumper_sha256":format!("{:x}",Sha256::digest(include_bytes!("../../reference/canon.c")))});
        let mut rust = json!({"profile":"engine-converted","producer":"rust-scene-v1","dumper_sha256":rust_scene_source_sha256()});
        assert!(validate_scene_receipts(&oracle, &rust).is_ok());
        rust["dumper_sha256"] = json!("0".repeat(64));
        assert!(validate_scene_receipts(&oracle, &rust).is_err());
        rust["dumper_sha256"] = json!(rust_scene_source_sha256());
        rust["profile"] = json!("historical-raw");
        assert!(validate_scene_receipts(&oracle, &rust).is_err());
        let mut bad_oracle = oracle;
        bad_oracle["dumper_sha256"] = json!("0".repeat(64));
        rust["profile"] = json!("engine-converted");
        assert!(validate_scene_receipts(&bad_oracle, &rust).is_err());
    }

    #[test]
    fn all_six_domains_and_counts_are_exact() {
        for path in [
            "/graph/id:1/visible",
            "/meshes/id:2/vertices",
            "/skins/id:3/weights",
            "/blend_shapes/id:4/position_offsets",
            "/materials/id:5/factor",
            "/curves/id:6/keyframes",
            "/meshes/length",
        ] {
            let a = f(1.0);
            let b = f(f64::from_bits(1.0f64.to_bits() + 1));
            let err = difference(path, &a, &b).unwrap_err();
            assert!(err.contains(path), "{err}");
        }
        assert!(difference("/meshes/length", &json!({"u64":"1"}), &json!({"u64":"2"})).is_err());
        assert!(difference("/meshes/id/values", &json!([f(1.0)]), &json!([])).is_err());
        assert!(difference("/curves/id/time", &f(0.0), &f(-0.0)).is_err());
        assert!(difference(
            "/dom/value",
            &f(f64::from_bits(0x7ff8000000000001)),
            &f(f64::from_bits(0x7ff8000000000002))
        )
        .is_err());
    }
    #[test]
    fn only_declared_evaluation_fields_get_tolerance() {
        let path = "/curve_samples/id:1/0/value";
        assert!(difference(path, &e(&[1.0]), &e(&[1.0 + 1e-9])).is_ok());
        assert!(difference(path, &e(&[1.0]), &e(&[1.0 + 1e-6])).is_err());
        assert!(difference(path, &e(&[1.0]), &e(&[1.0, 2.0])).is_err());
        for path in [
            "/curves/id/min_value",
            "/curves/id/keyframes/0/time",
            "/meshes/length",
            "/evaluated/default/0/nodes/length",
        ] {
            assert!(difference(path, &e(&[1.0]), &e(&[1.0])).is_err());
        }
        assert!(validate_value(path, &e(&[f64::NAN])).is_err());
        assert!(validate_value(path, &e(&[f64::INFINITY])).is_err());
    }
    #[test]
    fn quaternion_sign_and_angular_gate() {
        let path = "/evaluated/default/0/nodes/root/local/rotation";
        let q = |x: &[f64]| json!({"eval_quat": x.iter().map(|v| format!("{:016x}", v.to_bits())).collect::<Vec<_>>()});
        let a = q(&[0.0, 0.0, 0.0, 1.0]);
        assert!(difference(path, &a, &q(&[0.0, 0.0, 0.0, -1.0])).is_ok());
        assert!(difference(
            path,
            &a,
            &q(&[0.0, 0.0, (1e-8f64 / 2.0).sin(), (1e-8f64 / 2.0).cos()])
        )
        .is_ok());
        assert!(difference(
            path,
            &a,
            &q(&[0.0, 0.0, (1e-5f64 / 2.0).sin(), (1e-5f64 / 2.0).cos()])
        )
        .is_err());
        assert!(validate_value(path, &q(&[0.0; 4])).is_err());
    }
    #[test]
    fn rejects_ambiguous_or_rounded_json() {
        assert!(parse(b"{\"kind\":\"field\",\"kind\":\"footer\"}\n").is_err());
        assert!(parse(b"{\"f64\":0.1}\n").is_err());
        assert!(validate_value("/dom/x", &json!({"bytes_hex":"01","length":"2"})).is_err());
    }
}
