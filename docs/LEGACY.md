# Legacy FBX construction

The source reference is ufbx 0.23.1, hash-pinned in
[`reference/pins.json`](../reference/pins.json). The implementation preserves
source DOM nodes and translates the older scene layout into the existing scene
model. It applies the same import options as the 7.x path.

- **Names and connections.** `ufbxi_read_object` and `ufbxi_read_connections`
  use complete type/name strings before version 7000, including the reversed
  `Name\0\x01Type` binary spelling. `Model::Scene` denotes the implicit root.
  The Rust index uses bounded, sorted borrowed-name lookups rather than exposing
  generated numeric identifiers as authored IDs. Optional public source IDs
  are `None`; integer-only animation and unsupported-record ID fields use zero
  when the source format has no numeric ID.
- **Model attributes.** `ufbxi_read_synthetic_attribute` creates the embedded
  mesh, camera, light, bone or empty before reading its model. Properties in
  `ufbxi_node_prop_names`, and user properties, stay on the model; the others
  belong to the attribute. Property connections and skin connections are
  redirected to the attribute as in `ufbxi_resolve_connections`.
- **Takes.** `ufbxi_read_take` generates a stack and `BaseLayer` per take and
  synthesizes local/reference time properties. `ufbxi_read_take_prop_channel`
  flattens `Transform/T`, `/R`, `/S` to local translation, rotation and scaling.
  Component defaults survive even when they have no curve.
- **Keys.** `ufbxi_read_take_anim_channel` interprets the heterogeneous `Key`
  array: times, values, interpolation characters, slope modes and weight modes.
  Missing or non-positive `KeyVer` selects 4004 before version 6000 and 4005
  from version 6000, matching the reference fallback.
  It carries incoming slopes/weights to the next key, calculates automatic
  slopes with the same rules as the reference, and retains single-precision
  tangent handles. Legacy canonical sampling uses seconds neighbours and
  segment fractions; tick neighbours require an authored integer `KeyTime`
  array, as in `reference/canon.c:add_curve_samples`.
- **Properties and defaults.** `ufbxi_read_properties` prefers `Properties70`,
  then reads `Properties60` with its shorter property header. Templates remain
  a separate defaults chain. `ufbxi_read_legacy_settings` supplies frame rate
  properties from older settings blocks.
- **Geometry and deformers.** The 6.x path reads model-local layer elements
  through the same conversion as separate geometries. Nested `Shape` records
  create blend deformers, channels and targets as in
  `ufbxi_read_synthetic_blend_shapes`; pose references use names. The 5.8 root
  layout follows `ufbxi_read_legacy_model` and `ufbxi_read_legacy_mesh`, including
  scalar arrays, normal mapping and `GeometryUVInfo`.
- **Texture layers.** `ufbxi_read_mesh` records legacy texture-slot layers.
  `ufbxi_finalize_scene` resolves those slots through mesh/instance texture
  connections and patches otherwise untextured materials in material/property/slot
  order. The authored connection records remain intact.
- **Versions.** `ufbxi_read_header_extension` reconciles old binary container
  versions with the declared pre-6000 document version. A physical 3000
  container declaring 5800 is supported; a logical document older than 5800
  remains rejected.
- **Byte order.** Binary headers, scalar words and compressed/uncompressed
  array words use the declared byte order. Old heterogeneous key arrays use
  the same scalar decoder; no little-endian assumption is introduced.

The bounded conversion and fixtures are in `src/legacy.rs`,
`src/scene_animation.rs` and `tests/fbx6.rs`. `fuzz/make_legacy_seeds.py` generates
self-authored seeds for both byte orders, compressed geometry, mixed key
storage and the old root layout. Fuzz campaigns are a separate pre-release step.

`reference/compare_corpus.py` compares validated full streams, including
animation, deformers, evaluated samples and the `order` section, and separately
checks the public scene. Each receipt retains the input hash, producer exit
codes, exact first differences and the proof for every accepted retained-DOM
mutation. See [VERIFICATION.md](VERIFICATION.md) for qualification results.
