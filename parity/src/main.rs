use fbx_parity::{compare_files, compare_scene_files, rust_scene_source_sha256, validate};
use fbx_rs::canon_scene::{self, DumpLimits};
use std::{env, fs::File, io::BufReader, path::Path, process::ExitCode};

mod check_public;

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
    if args.len() == 2 && args[0] == "dump-scene" {
        let input = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
        let stdout = std::io::stdout();
        let out = std::io::BufWriter::new(stdout.lock());
        let limits = DumpLimits::default();
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
            "usage: fbx-parity validate FILE | dump-scene FBX | unsupported FBX | check-public FBX | compare [--allow-profile-mixing] EXPECTED ACTUAL | compare-scene ORACLE RUST"
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
