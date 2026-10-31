# Changelog

## 0.1.3 (unreleased)

- Add legacy FBX construction for name-based connections, embedded model
  attributes, Properties60, Takes and heterogeneous animation keys, older
  geometry layouts and both binary byte orders. Qualification is recorded in
  `docs/VERIFICATION.md`; the construction rules are in `docs/LEGACY.md`.
- Add a full-scene corpus comparison with strict typed list order, per-file
  receipts and proved retained-DOM mutation checks, plus authored legacy
  regression fixtures and reproducible fuzz seeds.

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

### Added

- `Limit` and `Error::LimitExceeded(Limit)`: a refusal caused by a `Limits`
  field now names that field (`Limit::field()`) instead of a free-text message.
  `Error::InputTooLarge` still reports `max_input_bytes`.
- `Limits` gains `max_meshes`, `max_layers`, `max_face_vertices` and
  `max_triangulation_work`. They were fixed constants on the load path, so every
  limit that can reject a file is now a `Limits` field.

### Changed

- Raised the `Limits` defaults so production-size exports load: `max_input_bytes`
  1 GiB (was 256 MiB), `max_nodes` 16,777,216 (was 1,048,576), `max_properties`
  33,554,432 (was 4,194,304), `max_array_elements` 134,217,728 (was
  67,108,864), `max_decompressed_bytes` 1 GiB (was 256 MiB), `max_allocation`
  4 GiB on 64-bit targets (was 512 MiB), `max_objects` 4,194,304,
  `max_connections` 8,388,608. A 355 MiB scene with 7.5 million DOM nodes loads
  in 16 s with a peak resident size of 3.8 GiB; the two Bistro interiors (40 and
  46 MiB) peak at 487 and 553 MiB. Peak resident size on the production files
  measured 10.9 to 12.2 times the input size, and each conversion stage charges
  at most `max_allocation`. Lower the fields for untrusted input.
  `Limits` has new fields, so struct-literal construction without
  `..Limits::default()` no longer compiles.
- `Limits::max_array_elements` is documented as the total over every array in
  the document, which is what the parser has always enforced.

### Fixed

- A layer element with a mapping mode the reader does not know, such as
  `ByControlPoint` on a normal layer, no longer fails the whole mesh. ufbx 0.23.1
  has no `ByControlPoint` alias: `ufbxi_read_vertex_element` accepts only
  `ByPolygonVertex`, `ByVertex`/`ByVertice`, `ByPolygon` and `AllSame`
  (ufbx.c:12797 and 12872) and ignores any other mapping with a warning
  (ufbx.c:12850 and 12903, `ufbxi_warn_polygon_mapping`), so the layer element is
  dropped and the mesh loads. fbx-rs now does the same for normals, tangents, UV
  sets, colors, materials and smoothing.
- Triangulation no longer charges the allocation budget quadratically: a mesh of
  160,000 quads was refused under a 4 GiB `max_allocation` because every polygon
  grew the output by exactly its own size and the budget charges the vector's
  whole old capacity on each growth. The output now grows geometrically.
- Reading the properties of one object cost time proportional to the whole
  document (a 7.5 million node file took longer than 20 minutes). A per-document
  index of children makes it proportional to the object's own nodes; output is
  unchanged on the corpus.
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
