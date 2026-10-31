use fbx_parity::{
    compare_files, compare_scene_files, rust_scene_source_sha256, validate, validate_with,
};
use fbx_rs::canon_scene::{self, DumpLimits};
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::ExitCode,
};

mod check_public;

/// `sections [--save DIR] [FILE]`: validate one complete stream (FILE, or stdin
/// when absent or `-`) in a single pass and print `name count sha256` per
/// section. Two streams are equal exactly when their lines are equal, so
/// comparing these lines compares whole sections without holding either stream;
/// `--save DIR` also writes each section's field lines to `DIR/NAME.jsonl` so
/// that a differing section can be examined afterwards.
fn sections(args: &[String]) -> Result<(), String> {
    let (save, file) = match args {
        [flag, dir, rest @ ..] if flag == "--save" && rest.len() <= 1 => {
            (Some(Path::new(dir.as_str())), rest.first())
        }
        rest if rest.len() <= 1 && !rest.iter().any(|a| a == "--save") => (None, rest.first()),
        _ => return Err("usage: fbx-parity sections [--save DIR] [FILE|-]".into()),
    };
    if let Some(dir) = save {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut reader: Box<dyn BufRead> = match file.map(String::as_str) {
        None | Some("-") => Box::new(BufReader::with_capacity(1 << 20, std::io::stdin().lock())),
        Some(path) => Box::new(BufReader::with_capacity(
            1 << 20,
            File::open(path).map_err(|e| format!("{path}: {e}"))?,
        )),
    };
    let mut current: Option<(String, BufWriter<File>)> = None;
    let receipt = validate_with(&mut reader, &mut |section, line| {
        let Some(dir) = save else { return Ok(()) };
        if current.as_ref().map(|(name, _)| name.as_str()) != Some(section) {
            if let Some((_, mut done)) = current.take() {
                done.flush().map_err(|e| e.to_string())?;
            }
            let file =
                File::create(dir.join(format!("{section}.jsonl"))).map_err(|e| e.to_string())?;
            current = Some((section.to_owned(), BufWriter::with_capacity(1 << 20, file)));
        }
        if let Some((_, w)) = current.as_mut() {
            w.write_all(line).map_err(|e| e.to_string())?;
        }
        Ok(())
    })?;
    if let Some((_, mut done)) = current.take() {
        done.flush().map_err(|e| e.to_string())?;
    }
    for ((name, count), hash) in receipt.sections.iter().zip(&receipt.hashes) {
        println!("{name} {count} {hash}");
    }
    println!("records {}", receipt.records);
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args: Vec<_> = env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "validate" {
        let file = File::open(&args[1]).map_err(|e| e.to_string())?;
        let receipt = validate(&mut BufReader::new(file))?;
        println!(
            "valid: {} fields, {} sections",
            receipt.records,
            receipt.sections.len()
        );
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "sections") {
        return sections(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "dump-scene") {
        let mut limits = DumpLimits::default();
        let mut rest = args[1..].to_vec();
        if rest.len() == 3 && rest[0] == "--max-output" {
            let bytes: u64 = rest[1]
                .parse()
                .map_err(|_| format!("--max-output: not a byte count: {}", rest[1]))?;
            // 0 lifts the cap: a whole stream of a production-size file is tens of GiB.
            limits.max_output = if bytes == 0 {
                usize::MAX
            } else {
                usize::try_from(bytes).map_err(|_| "--max-output exceeds usize".to_string())?
            };
            rest.drain(..2);
        }
        if rest.len() != 1 {
            return Err("usage: fbx-parity dump-scene [--max-output BYTES|0] FBX".into());
        }
        let input = std::fs::read(&rest[0]).map_err(|e| e.to_string())?;
        let stdout = std::io::stdout();
        let out = std::io::BufWriter::new(stdout.lock());
        let summary = canon_scene::dump(&input, out, &limits, &rust_scene_source_sha256())?;
        for w in &summary.warnings {
            eprintln!("warning: {w}");
        }
        return Ok(());
    }
    if args.len() == 2 && args[0] == "unsupported" {
        let input = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
        let limits = DumpLimits::default();
        let scene = canon_scene::project(&input, &limits)?;
        let mut counts = std::collections::BTreeMap::new();
        for (kind, _) in &scene.unsupported {
            *counts.entry(format!("{kind:?}")).or_insert(0u64) += 1;
        }
        println!(
            "{}",
            serde_json::to_string(&counts).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if args.len() == 2 && args[0] == "check-public" {
        let input = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
        let limits = DumpLimits::default();
        let canon = canon_scene::project(&input, &limits)?;
        let scene =
            fbx_rs::load(&input, &fbx_rs::LoadOptions::default()).map_err(|e| e.to_string())?;
        let n = check_public::check(&canon, &scene, &limits)?;
        println!("public-equal: {n} values");
        return Ok(());
    }
    if args.len() == 3 && args[0] == "compare-scene" {
        let receipt = compare_scene_files(Path::new(&args[1]), Path::new(&args[2]))?;
        println!(
            "equal: {} fields, {} sections",
            receipt.records,
            receipt.sections.len()
        );
        return Ok(());
    }
    let allow = args.get(1).is_some_and(|v| v == "--allow-profile-mixing");
    if allow {
        args.remove(1);
    }
    if args.len() != 3 || args[0] != "compare" {
        return Err(
            "usage: fbx-parity validate FILE | dump-scene [--max-output BYTES|0] FBX | sections [--save DIR] [FILE|-] | unsupported FBX | check-public FBX | compare [--allow-profile-mixing] EXPECTED ACTUAL | compare-scene ORACLE RUST"
                .into(),
        );
    }
    let receipt = compare_files(Path::new(&args[1]), Path::new(&args[2]), allow)?;
    println!(
        "equal: {} fields, {} sections",
        receipt.records,
        receipt.sections.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
