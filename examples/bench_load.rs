//! Process-level benchmark driver for the public load path: the input is read
//! once, then `load` (parse, index and static conversion into the public
//! `Scene` with the default `LoadOptions`) runs REPEATS times.
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
    let options = fbx_rs::LoadOptions::default();
    let mut nodes = 0;
    for _ in 0..repeats {
        match fbx_rs::load(black_box(&input), &options) {
            Ok(s) => nodes = black_box(s.nodes().len()),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    }
    println!("{{\"repeats\":{repeats},\"nodes\":{nodes}}}");
    ExitCode::SUCCESS
}
