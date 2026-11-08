#![no_main]
//! Scene cache reader robustness: `fbx_rs::cache::read` on arbitrary bytes
//! never panics and stays within a small allocation envelope, and every
//! sidecar it accepts has exactly one encoding: writing the read scene back
//! reproduces the input byte for byte.
//!
//! Seeds: run the `scene_cache` integration test with `FBX_CACHE_SEED_DIR`
//! set; it writes each fixture's (and corpus file's) sidecar under the key
//! below. The checksum trailer rejects most mutations, so seeds matter.
use fbx_rs::cache::{self, CacheKey};
use fbx_rs::{Limits, LoadOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(key) = CacheKey::new(b"fuzz-seed", &LoadOptions::default(), &Limits::default()) else {
        return;
    };
    let limits = Limits {
        max_allocation: 1 << 24,
        ..Limits::default()
    };
    if let Ok(scene) = cache::read(data, &key, &limits) {
        let mut out = Vec::new();
        scene.write_cache(&key, &mut out).expect("writing to a Vec");
        assert!(out == data, "accepted sidecar does not round-trip");
        std::hint::black_box((scene.nodes().len(), scene.element_count()));
    }
});
