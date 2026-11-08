//! Scene cache: a sidecar reads back the scene that wrote it, bit for bit;
//! any other key, a truncated or flipped byte and a hostile count are typed
//! errors, never a panic or a different scene.
//!
//! `FBX_CACHE_CORPUS_PATHS` names a file with one FBX path per line to add to
//! the fixtures. `FBX_CACHE_SEED_DIR` writes each sidecar there under the key
//! the `cache_read` fuzz target uses.
use fbx_rs::cache::{self, CacheKey, MissReason, ReadError};
use fbx_rs::{load_with_limits, BakeOptions, Limit, Limits, LoadOptions, Scene};
use std::path::PathBuf;

const RICH: &[u8] = include_bytes!("fixtures/allocation-rich.fbx");

fn digest(bytes: &[u8]) -> [u8; 8] {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    hash.to_le_bytes()
}

fn key_for(bytes: &[u8], limits: &Limits) -> CacheKey {
    CacheKey::new(&digest(bytes), &LoadOptions::default(), limits).unwrap()
}

fn write(scene: &Scene, key: &CacheKey) -> Vec<u8> {
    let mut out = Vec::new();
    scene.write_cache(key, &mut out).unwrap();
    out
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Rewrite the trailer so that only the structural checks can reject `bytes`.
fn reseal(bytes: &mut [u8]) {
    let end = bytes.len() - 8;
    let sum = adler32(&bytes[..end]);
    bytes[end..end + 4].copy_from_slice(&sum.to_le_bytes());
}

/// Byte offset of section `index` (0 = SCEN) from the section table.
fn section_offset(bytes: &[u8], index: usize) -> usize {
    let key_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let entry = 16 + key_len + 4 + index * 20;
    u64::from_le_bytes(bytes[entry + 4..entry + 12].try_into().unwrap()) as usize
}

fn inputs() -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "fbx"))
        .collect::<Vec<_>>();
    paths.sort();
    if let Some(manifest) = std::env::var_os("FBX_CACHE_CORPUS_PATHS") {
        paths.extend(
            std::fs::read_to_string(manifest)
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from),
        );
    }
    paths
}

/// Every truncation (at `step`) and every single-byte flip (at `step`) is a
/// miss or corrupt, never a scene.
fn assert_damage_rejected(bytes: &[u8], key: &CacheKey, step: usize) {
    for len in (0..bytes.len()).step_by(step) {
        let r = cache::read(&bytes[..len], key, &Limits::default());
        assert!(
            matches!(r, Err(ReadError::Miss(_) | ReadError::Corrupt)),
            "truncated to {len}: {:?}",
            r.err()
        );
    }
    let mut flipped = bytes.to_vec();
    for at in (0..bytes.len()).step_by(step) {
        flipped[at] ^= 0x5a;
        let r = cache::read(&flipped, key, &Limits::default());
        assert!(
            matches!(r, Err(ReadError::Miss(_) | ReadError::Corrupt)),
            "byte {at} flipped: {:?}",
            r.err()
        );
        flipped[at] = bytes[at];
    }
}

#[test]
fn sidecar_round_trips_bit_exact() {
    let limits = Limits::default();
    let seed_dir = std::env::var_os("FBX_CACHE_SEED_DIR").map(PathBuf::from);
    let (mut loaded, mut rejected) = (0, 0);
    for path in inputs() {
        let fbx = std::fs::read(&path).unwrap();
        let load_start = std::time::Instant::now();
        let Ok(scene) = load_with_limits(&fbx, &LoadOptions::default(), &limits) else {
            rejected += 1;
            continue;
        };
        let load_time = load_start.elapsed();
        loaded += 1;
        let key = key_for(&fbx, &limits);
        let first = write(&scene, &key);
        assert_eq!(write(&scene.clone(), &key), first, "{}", path.display());
        let read_start = std::time::Instant::now();
        let read = cache::read(&first, &key, &limits).unwrap();
        let read_time = read_start.elapsed();
        // write(read(write(load(x)))) == write(load(x)): every field survives,
        // including NaN payloads and negative zero, which `PartialEq` misses.
        assert!(write(&read, &key) == first, "{}", path.display());
        if fbx.len() > 16 << 20 {
            eprintln!(
                "{}: load {load_time:?}, cache read {read_time:?}, sidecar {} bytes",
                path.display(),
                first.len()
            );
        } else {
            assert_eq!(format!("{read:?}"), format!("{scene:?}"));
            for (stack, time) in [(None, 0.0), (None, 0.5), (Some(0), 1.0)] {
                assert_eq!(
                    format!("{:?}", read.evaluate(stack, time)),
                    format!("{:?}", scene.evaluate(stack, time))
                );
            }
            let bake = BakeOptions::default();
            assert_eq!(
                format!("{:?}", read.bake_animation(None, &bake)),
                format!("{:?}", scene.bake_animation(None, &bake))
            );
            // About 512 truncations and flips per file, covering every
            // header and section-table byte of the small fixtures.
            assert_damage_rejected(&first, &key, (first.len() / 512).max(1));
        }
        if let Some(dir) = &seed_dir {
            let fuzz_key = CacheKey::new(b"fuzz-seed", &LoadOptions::default(), &limits).unwrap();
            let name = format!("{}.sc", path.file_stem().unwrap().to_string_lossy());
            std::fs::write(dir.join(name), write(&scene, &fuzz_key)).unwrap();
        }
    }
    eprintln!("{loaded} round-tripped, {rejected} rejected by load");
    assert!(loaded >= 10);
}

#[test]
fn any_key_difference_is_a_miss() {
    let limits = Limits::default();
    let scene = load_with_limits(RICH, &LoadOptions::default(), &limits).unwrap();
    let key = key_for(RICH, &limits);
    let bytes = write(&scene, &key);
    let miss = |key: &CacheKey| cache::read(&bytes, key, &limits).err();
    assert_eq!(
        miss(&key_for(b"other", &limits)),
        Some(ReadError::Miss(MissReason::Key))
    );
    let looser = Limits {
        max_meshes: limits.max_meshes + 1,
        ..limits.clone()
    };
    assert_eq!(
        miss(&key_for(RICH, &looser)),
        Some(ReadError::Miss(MissReason::Key))
    );
    let options = LoadOptions {
        target_unit_meters: 0.01,
        ..LoadOptions::default()
    };
    let other = CacheKey::new(&digest(RICH), &options, &limits).unwrap();
    assert_eq!(miss(&other), Some(ReadError::Miss(MissReason::Key)));

    let mut version = bytes.clone();
    version[8] = 2;
    assert_eq!(
        cache::read(&version, &key, &limits).err(),
        Some(ReadError::Miss(MissReason::FormatVersion))
    );
    assert_eq!(
        cache::read(RICH, &key, &limits).err(),
        Some(ReadError::Miss(MissReason::NotACache))
    );
    assert!(CacheKey::new(&[], &LoadOptions::default(), &limits).is_err());
    assert!(CacheKey::new(&[0; 65], &LoadOptions::default(), &limits).is_err());
    assert!(CacheKey::new(&[0; 64], &LoadOptions::default(), &limits).is_ok());
}

#[test]
fn hostile_counts_are_refused_before_allocation() {
    let limits = Limits::default();
    let scene = load_with_limits(RICH, &LoadOptions::default(), &limits).unwrap();
    let key = key_for(RICH, &limits);
    let bytes = write(&scene, &key);
    // NODE starts with the node count; claim more nodes than bytes remain.
    let mut huge = bytes.clone();
    let node = section_offset(&huge, 1);
    huge[node..node + 8].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
    reseal(&mut huge);
    assert_eq!(
        cache::read(&huge, &key, &limits).err(),
        Some(ReadError::Corrupt)
    );
    // A non-canonical bool tag is corrupt even with a valid checksum.
    let mut tag = bytes.clone();
    let scen = section_offset(&tag, 0);
    // SCEN: element_count u64, then the axes (3 bytes) and unit f64; the
    // `Option<Axis>` tag follows at +19.
    tag[scen + 19] = 7;
    reseal(&mut tag);
    assert_eq!(
        cache::read(&tag, &key, &limits).err(),
        Some(ReadError::Corrupt)
    );
}

#[test]
fn reading_is_charged_to_the_allocation_limit() {
    let scene = load_with_limits(RICH, &LoadOptions::default(), &Limits::default()).unwrap();
    let small = Limits {
        max_allocation: 4096,
        ..Limits::default()
    };
    let key = key_for(RICH, &small);
    let bytes = write(&scene, &key);
    assert_eq!(
        cache::read(&bytes, &key, &small).err(),
        Some(ReadError::Limit(Limit::Allocation))
    );
    let ample = Limits::default();
    assert!(cache::read(&bytes, &key, &ample).is_ok());
}
