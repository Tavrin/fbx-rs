//! Process-level benchmark driver for parse+convert: the input is read once,
//! then `canon_scene::project` (syntax parse, object index, graph, transforms,
//! animation structure, meshes, materials, lights/cameras/bones) runs
//! REPEATS times. The canonical stream is not written.
use fbx_rs::canon_scene::{project, DumpLimits};
use std::{env, fs, hint::black_box, process::ExitCode};
fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        return ExitCode::from(2);
    };
    let Some(repeats) = args.next().and_then(|s| s.to_str()?.parse::<usize>().ok()) else {
        return ExitCode::from(2);
    };
    if repeats == 0 || repeats > 100_000 {
        return ExitCode::from(2);
    }
    let Ok(input) = fs::read(path) else {
        return ExitCode::from(2);
    };
    let limits = DumpLimits::default();
    let mut elements = 0;
    for _ in 0..repeats {
        match project(black_box(&input), &limits) {
            Ok(s) => elements = black_box(s.elements.len()),
            Err(_) => return ExitCode::from(1),
        }
    }
    println!("{{\"repeats\":{repeats},\"elements\":{elements}}}");
    ExitCode::SUCCESS
}
