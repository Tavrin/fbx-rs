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

### Fixed

- Every `Scene` list now has the order ufbx 0.23.1 exposes, and the order is
  documented as part of the public contract (crate documentation, "Order of the
  lists"). Objects keep file order instead of source-ID order (`materials`,
  `meshes`, `textures`, `cameras`, `lights`, bones, deformers, poses and the
  animation stacks, layers, values and curves), and `nodes` are sorted by depth,
  parent position, scale helpers first, then file position. Node indices in
  `evaluate`, `evaluate_transform` and `BakedNode::node` follow the new order.
  This changes the indices of existing scenes: stored indices from 0.1.1 do not
  carry over.
- `bake_animation` now matches ufbx for scale helpers: baking a helper also
  bakes its otherwise unanimated sibling nodes, dependants are resampled at the
  helper's baked keys and evaluated from them, nodes are baked in node order and
  baked properties are ordered by object.

### Verification

- The canonical stream ends with an `order` section recording each typed list,
  and `reference/compare_order.py` and `reference/compare_bake.py` compare list
  and bake order. The earlier "316/316 bit-exact" claim compared values by
  identity only; see `docs/VERIFICATION.md` for what it did and did not cover.

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
