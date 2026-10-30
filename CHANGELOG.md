# Changelog

## 0.1.2 (unreleased)

- Make every allocation on the default loading and evaluation paths fallible,
  including converter graphs, geometry, materials, deformers and ordered
  property indexes. Allocator refusal returns typed `Error::Allocation`.
  Real allocator sweeps cover ASCII/compressed binary load, evaluation, bake
  and error reporting; see `docs/VERIFICATION.md` for the contract boundary.
- Materials retain computed feature flags, authored/template source properties,
  and every texture attachment. Textures expose ordered layers and flattened
  file-texture links.
- `Scene::bake_animation()` adds bounded, independently reduced transform and
  property channels, tick-based start trimming and original stack durations.
  Compatibility with the complete ufbx bake is not claimed; see the verification
  notes for the tested option subset and remaining differences.


All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-10-30

### Added

- Bones, bind/rest poses, skin deformers and clusters with all control-point
  weights, skinning methods and dual-quaternion blend factors.
- Blend-shape channels with sparse position/normal offsets and in-between targets.
- Animation stacks, layers, property bindings and curves with constant, linear
  and weighted cubic interpolation, tangent reconstruction and extrapolation.
- Scene, transform and property evaluation, including blend-shape weights and
  skin matrices.

### Changed

- Bound deformer and animation allocations, traversal and sorting work;
  index connection, pose, cluster and mesh lookups.

### Fixed

- Reject oversized sparse indices, invalid or missing pose IDs, malformed
  deformer counts, unsorted or nonfinite curve times, invalid curve attribute
  counts and excessive key counts.
- Preserve unknown source axes when global axis properties are absent.

### Deliberate differences from ufbx

- Keep Blender per-offset weights in the same order as sorted sparse offsets,
  preserving each weight's association with its vertex.
- Reject animation-connection cycles, including disconnected cycles and self-links.

[Unreleased]: https://github.com/Tavrin/fbx-rs/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/Tavrin/fbx-rs/compare/v0.1.0...v0.1.1
