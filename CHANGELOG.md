# Changelog

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
