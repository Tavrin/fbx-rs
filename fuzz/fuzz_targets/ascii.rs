#![no_main]
use fbx_rs::syntax::{parse, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_depth: 16,
        max_nodes: 256,
        max_properties: 1024,
        max_array_elements: 4096,
        max_decompressed_bytes: 32768,
        max_string_length: 8192,
        max_allocation: 131072,
    };
    let _ = parse(data, &limits);
});
