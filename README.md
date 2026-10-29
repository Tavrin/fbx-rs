# fbx-rs

A pure-Rust loader for FBX files. It reads binary and ASCII FBX 7.x files
into an owned, static scene: nodes, meshes, materials, textures, cameras and
lights. Every value it returns is the value [ufbx](https://github.com/ufbx/ufbx)
0.23.1 produces for the same file and import options, checked bit for bit on
a test corpus.

## What it does

- Reads binary and ASCII FBX, file versions 7100 to 7700 (FBX 7.1 to 7.7).
- Converts the static scene: the node hierarchy with local and world
  transforms; meshes with positions, normals, every UV set (with tangents and
  bitangents), color sets, faces, per-face materials and a triangulated index
  buffer; materials (FBX and PBR maps); textures (paths as written and
  embedded content); cameras; lights; scene settings.
- Converts to right-handed Y-up with 1 m units.
- Bounds every allocation with configurable limits, and contains no `unsafe`
  code. It builds for `wasm32-unknown-unknown`.

## What it does not do

- Skins, blend shapes and animation are not converted. They are detected and
  listed in `Scene::unsupported()`, so you can tell what was left out. NURBS
  geometry, caches, constraints and audio are reported the same way.
- FBX 6.1 and older files are rejected with an error.
- Other target axes or units are not implemented yet; asking for them
  returns `Error::UnsupportedOptions`.
- It does not write FBX, decode images or load external files: texture paths
  are returned as written.

## Install

```toml
[dependencies]
fbx-rs = "0.1"
```

## Usage

```rust
let bytes = std::fs::read("model.fbx")?;
let scene = fbx_rs::load(&bytes, &fbx_rs::LoadOptions::default())?;
for node in scene.nodes() {
    if let Some(index) = node.mesh() {
        let mesh = &scene.meshes()[index];
        println!("{}: {} triangles", node.name(), mesh.triangles().len() / 3);
    }
}
for item in scene.unsupported() {
    println!("not converted: {:?}", item.kind);
}
```

`fbx_rs::load_with_limits` takes explicit `Limits` for untrusted input. The
lower-level `fbx_rs::syntax::parse` returns the raw node/property tree.

## Status and limits

Version 0.1 covers static scenes. On a corpus of 340 FBX files (see
[docs/VERIFICATION.md](docs/VERIFICATION.md)), 306 static files load with
results equal to ufbx 0.23.1, the 10 files that use rigs or animation differ
only in that content (which is reported as unsupported), and no difference is
unexplained. The 24 remaining files are FBX 6.1 or older and
are rejected. Camera and light modes that the corpus does not contain are
ported but not exercised. The parser and loader are fuzzed with libFuzzer.

Planned next: skinning, blend shapes and animation.

## Comparison with ufbx

ufbx is a C library that loads far more of FBX (animation, skinning, other
file formats, many options). fbx-rs covers static scenes only, in safe Rust
without a C toolchain. On the static content it returns the same values,
bit for bit, as described above.

Speed, measured as instructions on the 316 supported corpus files: a full
`load` executes about 2.2 times the instructions of a ufbx load with the same
options, partly because `load` also triangulates every mesh. Parsing alone
costs about the same as a whole ufbx load. Peak process memory per file was at most
4.4 MiB for fbx-rs and 3.4 MiB for ufbx on that corpus.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Parts of the conversion logic are
ported from ufbx (MIT); see [THIRD-PARTY.md](THIRD-PARTY.md).

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this crate by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
