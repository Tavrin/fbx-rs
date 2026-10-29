//! Syntax DOM stream (`fbx-syntax-dump/2`) for the corpus equivalence gate.
//!
//! Source property types are preserved; `reference/dom_equivalence.py` projects
//! them onto ufbx's retained DOM. Numbers are exact (decimal integers, IEEE bits
//! as hex). Binary array elements are big-endian hex, concatenated.
use fbx_rs::syntax::{parse, AsciiValue, Limits, Property};
use serde_json::{json, Value};
use std::{env, fs, io::Write, process::ExitCode};

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

fn ascii_value(v: &AsciiValue<'_>) -> String {
    match v {
        AsciiValue::I64(n) => format!("i{n}"),
        AsciiValue::F64(bits) => format!("f{bits:016x}"),
        AsciiValue::Bare(word) => format!("w{}", hex(word)),
        AsciiValue::NegativeZero => "z".to_owned(),
    }
}

fn prop(p: &Property<'_>) -> Value {
    match p {
        Property::I16(x) => json!({"t":"Y","i":x.to_string()}),
        Property::Bool(x) => json!({"t":"C","i":x.to_string()}),
        Property::BoolAlt(x) => json!({"t":"B","i":x.to_string()}),
        Property::I32(x) => json!({"t":"I","i":x.to_string()}),
        Property::I64(x) => json!({"t":"L","i":x.to_string()}),
        Property::F32(x) => json!({"t":"F","x":format!("{x:08x}")}),
        Property::F64(x) => json!({"t":"D","x":format!("{x:016x}")}),
        Property::String(x) => json!({"t":"S","x":hex(x)}),
        Property::Raw(x) => json!({"t":"R","x":hex(x)}),
        Property::AsciiBare(x) => json!({"t":"W","x":hex(x)}),
        Property::AsciiNegativeZero => json!({"t":"Z"}),
        Property::AsciiArray(values) => {
            let v = values.iter().map(ascii_value).collect::<Vec<_>>().join(",");
            json!({"t":"A","v":v})
        }
        Property::Array {
            kind,
            count,
            big_endian,
            bytes,
        } => {
            let width = match kind {
                b'f' | b'i' => 4,
                b'd' | b'l' => 8,
                _ => 1,
            };
            let mut data = bytes.to_vec();
            if !big_endian {
                for chunk in data.chunks_exact_mut(width) {
                    chunk.reverse();
                }
            }
            json!({"t":"a","k":(*kind as char).to_string(),"n":count,"x":hex(&data)})
        }
    }
}

fn main() -> ExitCode {
    let Some(path) = env::args_os().nth(1) else {
        return ExitCode::from(2);
    };
    let Ok(input) = fs::read(path) else {
        return ExitCode::from(2);
    };
    let doc = match parse(&input, &Limits::default()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e:?}");
            return ExitCode::from(1);
        }
    };
    let binary = input.starts_with(b"Kaydara FBX Binary  \x00\x1a");
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let header = json!({"schema":"fbx-syntax-dump/2","version":doc.version,
        "format": if binary { "binary" } else { "ascii" }});
    if writeln!(out, "{header}").is_err() {
        return ExitCode::from(2);
    }
    let mut depths: Vec<usize> = Vec::with_capacity(doc.nodes.len());
    for node in &doc.nodes {
        // Parents always precede children, so depth is one lookup.
        let depth = node.parent.map_or(0, |i| depths[i] + 1);
        depths.push(depth);
        let values = doc.properties[node.properties.clone()]
            .iter()
            .map(prop)
            .collect::<Vec<_>>();
        let line = json!({"depth":depth,"name_hex":hex(node.name),"values":values});
        if writeln!(out, "{line}").is_err() {
            return ExitCode::from(2);
        }
    }
    if out.flush().is_err() {
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
