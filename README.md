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
- Reads bones, bind/rest poses, skin clusters and all control-point influences,
  skinning methods and dual-quaternion blend factors; blend-shape channels,
  sparse targets and in-between weights at the stored deformation percentage.
- Converts to right-handed Y-up with 1 m units.
- Bounds every allocation with configurable limits, and contains no `unsafe`
  code. It builds for `wasm32-unknown-unknown`.

## What it does not do

- Animation curves and evaluation are not converted. They are listed in
  `Scene::unsupported()`, as are NURBS geometry, caches, constraints and audio.
  Mesh positions remain undeformed; the API exposes the stored skin and blend
  data for callers to apply.
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

The unreleased deformer API reads skin weights by control point, rather than
polygon corner. Each
vertex's influence range includes every stored weight, without normalization
or a maximum-influence reduction:

```rust
# fn read_weights(scene: &fbx_rs::Scene) {
for mesh in scene.meshes() {
    for &index in mesh.skin_deformers() {
        let skin = &scene.skin_deformers()[index];
        for (vertex, range) in skin.vertices.iter().enumerate() {
            for influence in &skin.weights[range.weight_begin..range.weight_begin + range.num_weights] {
                let cluster = &scene.skin_clusters()[skin.clusters[influence.cluster_index]];
                println!("vertex {vertex}: bone {:?}, weight {}", cluster.bone_node, influence.weight);
            }
        }
    }
    for &index in mesh.blend_deformers() {
        for &channel in &scene.blend_deformers()[index].channels {
            let channel = &scene.blend_channels()[channel];
            for target in &channel.keyframes {
                let shape = &scene.blend_shapes()[target.shape];
                println!("{}: {} offsets, contribution {}", channel.name,
                         shape.offset_vertices.len(), target.effective_weight);
            }
        }
    }
}
# }
```

`fbx_rs::load_with_limits` takes explicit `Limits` for untrusted input. The
lower-level `fbx_rs::syntax::parse` returns the raw node/property tree.

## Status and limits

The published 0.1 release covers static scenes; the unreleased source also
exposes stored deformers. On a corpus of 340 FBX files (see
[docs/VERIFICATION.md](docs/VERIFICATION.md)), 308 files have results equal to
ufbx 0.23.1, including all 306 original static files. Eight files differ only
in animation, and no difference is unexplained. Stored skin and blend-shape
records match on all seven rig fixtures. The 24 remaining files are FBX 6.1
or older and are rejected. Camera and light modes that the corpus does not contain are
ported but not exercised. The parser and loader are fuzzed with libFuzzer.

Planned next: animation curves and evaluation.

## Comparison with ufbx

ufbx is a C library that loads far more of FBX (animation, skinning, other
file formats, many options). fbx-rs covers static scenes and stored deformers, in safe Rust
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
