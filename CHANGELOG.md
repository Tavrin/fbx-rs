# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Animation stacks, layers and property bindings, authored curves with constant,
  linear and weighted cubic interpolation, tangent reconstruction and extrapolation.
- `Scene::evaluate`, `evaluate_transform`, `evaluate_property` and `evaluate_binding`
  for scene poses, node transforms, arbitrary properties and generated helper bindings.
- Evaluated blend-shape weights and skin matrices, animated scaling helpers,
  bounded malformed-curve checks and canonical curve and scene sampling.

- Owned bones and bind/rest poses, skin deformers and clusters with all
  control-point weights, skinning methods and dual-quaternion blend factors.
- Blend deformers, channels, sorted sparse position/normal offsets and
  in-between targets with effective weights at the stored deformation percentage.
- Mesh-to-deformer and node-to-bone/pose references, canonical deformer records,
  malformed-input tests and additional public-API fuzz invariants.

- `load` and `load_with_limits`: read a binary or ASCII FBX 7.1 to 7.7 file
  into an owned static `Scene` (nodes and transforms, meshes with UV and
  color sets and triangulation, materials, textures, cameras, lights,
  settings), converted to right-handed Y-up with 1 m units.
- `Scene::unsupported` lists content that is not converted.
- `syntax::parse`: bounded binary and ASCII FBX 7.x parser returning the
  node/property tree. FBX 6.1 and older are rejected.
- `Limits` on input size, nesting, node, property, array and allocation
  counts for untrusted input.

### Fixed

- Reject oversized sparse indices before they can alias valid vertices.
- Bound deformer allocations before decoding and charge traversal/sorting work;
  index connection deduplication and pose/cluster lookups.
- Keep Blender per-offset weights parallel to sorted sparse offsets.
- Index legacy shape discovery and mesh sources; attach mesh deformers in one pass
  and avoid intermediate encoding copies when reading numeric ASCII arrays.
- Preserve unknown source axes when global axis properties are absent.
