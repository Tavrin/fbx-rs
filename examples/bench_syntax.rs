//! Process-level benchmark driver; input is read before repeated parses.
use fbx_rs::syntax::{parse, Limits};
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
    let limits = Limits::default();
    let mut nodes = 0;
    for _ in 0..repeats {
        match parse(black_box(&input), &limits) {
            Ok(d) => nodes = black_box(d.nodes.len()),
            Err(_) => return ExitCode::from(1),
        }
    }
    println!("{{\"repeats\":{repeats},\"nodes\":{nodes}}}");
    ExitCode::SUCCESS
}
