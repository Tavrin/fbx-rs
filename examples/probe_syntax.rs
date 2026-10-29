//! Corpus diagnostic; reads one input and reports the experimental parser result.
use fbx_rs::syntax::{parse, Limits};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let Some(path) = env::args_os().nth(1) else {
        return ExitCode::from(2);
    };
    let Ok(input) = fs::read(path) else {
        return ExitCode::from(2);
    };
    match parse(&input, &Limits::default()) {
        Ok(doc) => {
            println!(
                "ok nodes={} properties={}",
                doc.nodes.len(),
                doc.properties.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{error:?}");
            ExitCode::from(1)
        }
    }
}
