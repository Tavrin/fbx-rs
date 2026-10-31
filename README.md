# fbx-rs

A pure-Rust loader for FBX files. It reads binary and ASCII FBX 7.x files
into an owned scene: nodes, meshes, materials, textures, cameras and
lights, skins, blend shapes and animation. On the verification corpus, all 316
supported files match [ufbx](https://github.com/ufbx/ufbx) 0.23.1 with the same
import options: every value bit for bit (matched by object identity, with the
differences in [docs/VERIFICATION.md](docs/VERIFICATION.md) explained), and the
order of every list identical.

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
- Reads animation stacks, layers, property bindings and constant, linear or cubic
  curves with weighted tangents and extrapolation; evaluates node transforms,
  blend-shape weights and skin poses at any finite time.
- Converts to right-handed Y-up with 1 m units.
- Lists the nodes, materials, meshes and every other collection in the order
  ufbx does (file order of the objects; nodes by depth, then parent, then file
  position). The order is part of the public contract; see the crate
  documentation, "Order of the lists".
- Bounds every allocation with configurable limits, and contains no `unsafe`
  code. It builds for `wasm32-unknown-unknown`.

## What it does not do

- NURBS geometry, caches, constraints and audio are listed in
  `Scene::unsupported()`. Mesh positions remain undeformed; the API exposes the stored skin and blend
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

```no_run
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
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Reading skins and animation

Skin influences are indexed by control point. Each range contains all stored
weights, without normalization or a limit on influences. Animation time is in
seconds; `None` selects the default stack, and a stack index selects a named take.

```rust
# fn read_skin_and_animation(scene: &fbx_rs::Scene) -> Result<(), fbx_rs::Error> {
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
}
let pose = scene.evaluate(None, 0.5)?;
for (node, evaluated) in scene.nodes().iter().zip(&pose.nodes) {
    println!("{} at 0.5 seconds: {:?}", node.name(), evaluated.node_to_world);
}
# Ok(())
# }
```

`Scene::animation_stacks`, `animation_layers`, `animation_values` and
`animation_curves` expose the authored data. `evaluate_property` returns an
object property's numeric and retained nonnumeric values; `evaluate_binding`
also addresses generated scaling helpers. Evaluation leaves loaded data unchanged.

`fbx_rs::load_with_limits` takes explicit `Limits` for untrusted input. Every
limit that can reject a file is a field of `Limits`, documented with its
default, and a refusal returns `Error::LimitExceeded(Limit)` naming the field
(`Limit::field()`), or `Error::InputTooLarge` for `max_input_bytes`. The
defaults accept production-size exports (a 355 MiB scene with 7.5 million DOM
nodes loads with them, at a peak of about 11 times the file size); lower them
for untrusted input. The lower-level
`fbx_rs::syntax::parse` returns the raw node/property tree.

## Status and limits

Version 0.1.1 covers static scenes, stored deformers and animation evaluation.
On a corpus of 340 FBX files (see [docs/VERIFICATION.md](docs/VERIFICATION.md)),
all 316 supported files match ufbx 0.23.1 bit for bit by object identity,
including authored curves and evaluated samples, and the order of nodes,
materials, meshes, textures, cameras, lights, bones, deformers, poses and
animation lists is identical. Baked animation matches ufbx on the 8 animated
corpus files and 4 generated fixtures under four option sets, with node and
property order compared. Deliberate differences for malformed inputs and
Blender weight order are described in that verification report.
The 24 remaining files are FBX 6.1 or older and are rejected. Camera and light
modes that the corpus does not contain are ported but not exercised. The parser
and loader are fuzzed with libFuzzer; the latest animation changes have regression
tests and differential checks, without a new fuzz campaign.

## Comparison with ufbx

ufbx is a C library that loads far more of FBX (other file formats, surface
geometry, caches and many options). fbx-rs covers scenes, stored deformers and
animation evaluation, in safe Rust without a C toolchain. On the covered corpus,
its values match bit for bit, as described above.

Historical measurements, taken before animation evaluation was added, used the
316 supported corpus files. A full `load` executes about 2.2 times the instructions
of a ufbx load with the same
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


Material inspection includes `Material::features()`, `source_property(b"Opacity")`
(with source-template fallback), and `textures()` for every attachment, including
unmapped property names. `Texture::layers()` preserves blend modes and alpha;
`file_textures()` exposes flattened unique file texture indices.

`Scene::bake_animation(stack, &BakeOptions::default())` produces independently
reduced translation, quaternion, scale and numeric property channels. Baking is
fallible, with aggregate candidate-key, allocation and evaluation-work limits.
The default candidate-key cap is 16 million. Set `trim_start_time` to shift keys
using FBX ticks; playback bounds and duration retain the original stack range.
This is a subset of ufbx's baking controls. See [verification](docs/VERIFICATION.md)
for the compatibility boundary and for how baked output is compared with ufbx.
