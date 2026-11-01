# Verification

For the scene content it converts, fbx-rs targets the same values, in the same
order of every list, as ufbx 0.23.1 loaded with the same import options, with
the deliberate edge case differences below. This page says how
that is checked, on which files, with what result, and what is not covered.

## How equivalence with ufbx is checked

Both sides write the same canonical text format, `fbx-scene-canon/1`
([reference/SCHEMA.md](../reference/SCHEMA.md)): JSON Lines with a fixed
member order, every floating-point value written as its exact bit pattern,
and sections for settings, the retained DOM, elements, connections, the node
graph, meshes, stored skins and blend shapes, materials, cameras and lights,
triangles, authored curves and evaluated animation samples.

- The oracle is `reference/canon.c`, linked against the hash-pinned ufbx C
  sources (`reference/pins.json`) and built by `reference/build.py`. Its load
  options (`reference/options-engine-converted.json`) are those of an existing
  engine's ufbx-based importer: right-handed Y-up, 1 m units, geometry-modifying
  space conversion, compensated inheritance. These are the options fbx-rs
  implements.
- The Rust side is `fbx-parity dump-scene`, built from the same converter code
  as `fbx_rs::load` (crate `parity/`).
- `fbx-parity compare-scene` validates both streams completely (pins,
  coverage, options, section order, counts, checksums) and then compares them
  value by value. Its standalone evaluated comparison supports tolerances;
  the per-file gate below compares the raw bit strings directly.
- The `order` section is the only part of the stream that records sequence.
  Every other section is keyed by object identity (source ID and DOM ordinal,
  or an owner-derived key for generated elements), and lists such as children
  or material slots hold references to those keys. `order` lists, for every
  element type, the elements in the order ufbx exposes the typed list
  (`nodes`, `materials`, `anim_stacks`, and the rest of `elements_by_type`).
  The Rust side writes it from the same ordering that the public `Scene`
  slices use, and the comparison requires it to be identical, record for
  record.
- Each compared path is checked under these rules:
  - DOM mutation: ufbx modifies some retained DOM arrays in place while
    loading (`Geometry/Vertices`, `LayerElementNormal/Normals`, blend-shape
    arrays). Such a difference is accepted only when the oracle's DOM array
    equals, bit for bit, the oracle's own converted mesh data. This is proved
    per array.
  - Retained shape-array mutation: accepted only with bit identity against
    the oracle's own loaded offsets.
  - Animation is compared without an exemption: curve keys, tangent handles,
    layer bindings, property samples, node transforms, morph weights and
    skin matrices must all match bit for bit.
  - Reported axes: ufbx retains the source axes in `settings.axes`, while the
    public scene reports the converted target axes. The corpus comparator
    requires an exact source-property proof and the fixed target enums.
  - Unexplained: anything else. The target is zero.
- `fbx-parity check-public` then compares every value the public `Scene`
  exposes (nodes, transforms, meshes, bones, skins, poses, blend shapes,
  material maps, texture content, cameras, lights, animation records, evaluated
  poses and counts) with the canonical records of the same file, bit for bit,
  so the public API and the compared stream cannot drift apart. The records
  are paired with the `Scene` slices in the order written to the `order`
  section, so the public slices are tied to the order that is compared with
  ufbx.
- The syntax layer is checked separately: `reference/dom_equivalence.py`
  projects the parser's node/property tree onto ufbx's retained DOM (the
  rules cite ufbx source lines) and compares every value bit for bit.
- The list of unsupported objects (`Scene::unsupported`) is compared per
  file with the oracle's element counts per type.

No numeric tolerance is used for the per-file results below, including evaluated
quaternions and matrices. The reference sampling contract is in `reference/canon.c`;
the Rust emitter implements the same times and provenance. No oracle
nondeterminism was observed or used to justify a tolerance.

The per-file classification script is `reference/compare_corpus.py`. Its
fault tests require strict animation and order checks, input-hash binding, and
exact oracle-side retained-array proofs. Integer storage signedness is ignored
only inside the shape-index proof; compared scene fields remain exact.

## What the per-file comparison did not cover, until order was added

The earlier statement "316/316 bit-exact" was true of the values it compared and
silent about sequence. Measured with the `order` section against the previous
public lists, 182 of the 316 files had at least one list in a different order
from ufbx: materials in 165 files, nodes in 29, meshes in 27, textures and videos
in 3 each, bones in 3, animation curves and values in 5 each, and single files
for skin clusters, blend channels, animation layers and animation stacks. Three
properties of the checks hid it:

- The canonical stream is keyed by identity. Its element table is sorted by
  source ID and ordinal, every record sits under its own key, and every
  cross-reference is a key, so permuting a list left the stream unchanged. The
  stream deliberately never carries ufbx's dense typed indices.
- `check-public` paired the public `Scene` slices with the canonical records of
  the same converter by position. Both followed source-ID order, so they agreed
  with each other and not with ufbx.
- The bake comparison normalised node and property record order and replaced
  typed indices by source identities before comparing.

The `order` section, the strict `check-public` pairing and
`reference/compare_bake.py` (which requires the sequence of baked nodes and
properties to match line for line) close those gaps. `reference/compare_order.py`
runs the stream comparison of the `order` section over a list of files.

ufbx's rules, as implemented: typed lists keep creation order, which is file
order of the objects with generated elements after them; `nodes` are sorted by
depth, then by the creation order of the parent, then scale helpers before
ordinary siblings, then creation order.

## Corpus

The corpus is not stored in this repository. It holds 340 distinct files
(480 paths), chosen by provenance and licence:

| Source | Files | Licence |
| --- | ---: | --- |
| Kenney Furniture Kit (FBX format) | 140 | CC0-1.0 |
| Quaternius Ultimate Nature Pack (June 2019 download) | 150 | CC0-1.0, per the notice shipped with that download |
| ufbx repository test data (exporter regressions and embedded/external media) | 49 | MIT OR Unlicense |
| Self-authored regression (ordering and colour), generated by `reference/qualify.py` | 1 | MIT OR Apache-2.0 |

316 files are FBX 7.x (versions 7100 to 7700) in binary and ASCII form; 24
are FBX 5.8/6.x and were rejected in the previous qualification. Tags in the corpus manifest
mark files with skins, blend shapes or animation; the static files have none.

## Last completed FBX 7.x results

These detailed modern results are retained from the earlier qualification.
The final 347-input comparison below rechecks all 316 modern files, including
order and the public scene, and the 48 baked streams also pass again.

On ufbx 0.23.1 with the options above:

| Check | Result |
| --- | --- |
| Original static files (no rig or animation tags) | 306 of 306 equal: 43 with no difference at all, 263 differing only by proved in-place DOM mutation (460 arrays) |
| Stored skin and blend-shape records | 7 of 7 rig files bit-exact, including both FBX 7.1 nested-shape encodings |
| Whole-file classification, values matched by object identity | 316 equal (including proved DOM mutation), 0 deferred, 0 unexplained |
| Order of every typed list (`order` section) | 316 of 316 files identical to ufbx, 3,165 records over every non-empty list (nodes, meshes, materials, textures, cameras, lights, bones, deformers, poses, animation stacks, layers, values, curves) |
| Authored animation and evaluated samples | 8 animated files, 121 curves, all bit-exact; key/tick neighbours, segment fractions, take bounds and a 30 Hz grid |
| Baked animation (`reference/compare_bake.py`) | 12 inputs (the 8 animated corpus files and 4 generated fixtures), 4 option sets each (reduced, reduced and trimmed, reduced with rotation-aware reduction and trimmed, raw and trimmed): 48 of 48 streams identical, with baked node and property order and every key compared bit for bit; the scale-inheritance file `maya_scale_no_inherit_7700_ascii.fbx` included |
| Additional animation fixtures | 39 complete streams bit-exact: interpolation, weighted tangents, extrapolation, all rotation orders, layers, animated layer weights and recursive scale helpers |
| Unexplained differences | 0 files |
| Legacy FBX 5.8/6.x | 24 of 24 equal, including order; final per-file results below |
| Public `Scene` against canonical records, paired in ufbx list order | 316 of 316 files equal, 4,134,768 values, including authored animation and evaluated poses |
| Syntax layer against ufbx's retained DOM | 315 equal, 1 explained (ufbx sorts blend-shape offsets in place), 0 failing |
| Unsupported-object report against oracle counts | 316 files, 0 mismatches |
| wasm32 | canonical dump, public scene and evaluated poses identical to native on one animated morph fixture (Wasmtime); default-feature library build passes |

## Legacy qualification

The final comparison processes 347 distinct inputs: all 340 corpus files and
seven self-authored legacy fixtures. All 347 match ufbx 0.23.1 in every section
apart from the documented retained-DOM and reported-axes classes. There are
zero import errors and zero unexplained differences. Every typed-list order is
identical, and all 347 public scene checks and both producers' stream validation
checks pass. The modern corpus remains 316 of 316 equal, including order;
baked animation remains 48 of 48 exact streams across 12 inputs and four option
sets. Both additional Curtains files retain the results below.

The corpus contains 24 distinct legacy files. The additional requested list
contains 21 of those exact paths; inputs are read in place and bound to SHA-256.
The table reports each final full-stream comparison. **Equal** includes only
proved retained-DOM mutations and the documented source-axes versus target-axes
distinction. DOM counts are differing array fields, not individual scalars.
For the two Z-up Max 6.1 files, the two reported axis fields are accepted only
when the oracle enums reconstruct exactly from identical authored axis/sign
properties and the Rust enums equal the fixed import target. Corrupt, missing
or mismatched evidence fails; no graph, geometry, animation or order field is
exempted. Seven comparator fault tests cover the retained-array and axes proofs,
input binding, validation framing and strict animation/order checks.

| Legacy input | Final result | Proved DOM fields | Reported axes fields |
| --- | --- | ---: | ---: |
| `blender_279_default_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `blender_279_sausage_6100_ascii.fbx` | Equal; order exact | 2 | 0 |
| `blender_279_uv_sets_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `max2009_cube_anim_5800_ascii.fbx` | Equal; order exact | 2 | 0 |
| `max2009_cube_anim_5800_binary.fbx` | Equal; order exact | 2 | 0 |
| `max2009_cube_anim_6100_ascii.fbx` | Equal; order exact | 2 | 2 |
| `max2009_cube_anim_6100_binary.fbx` | Equal; order exact | 2 | 2 |
| `maya_blend_shape_cube_6100_ascii.fbx` | Equal; order exact | 4 | 0 |
| `maya_blend_shape_cube_6100_binary.fbx` | Equal; order exact | 4 | 0 |
| `maya_camera_light_axes_y_up_6100_ascii.fbx` | Equal; order exact | 0 | 0 |
| `maya_camera_light_axes_y_up_6100_binary.fbx` | Equal; order exact | 0 | 0 |
| `maya_cube_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `maya_cube_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `maya_cube_big_endian_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `maya_interpolation_modes_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `maya_interpolation_modes_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `maya_pivots_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `maya_pivots_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `maya_scale_no_inherit_6100_ascii.fbx` | Equal; order exact | 0 | 0 |
| `maya_uv_sets_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `maya_uv_sets_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `maya_textured_cube_6100_ascii.fbx` | Equal; order exact | 1 | 0 |
| `maya_textured_cube_6100_binary.fbx` | Equal; order exact | 1 | 0 |
| `self_authored_order_material_6100_ascii.fbx` | Equal; order exact | 0 | 0 |

The authored fixtures cover named takes, model-local meshes, Properties60,
legacy root geometry/UVs, little- and big-endian mixed keys and compressed
geometry, named skin/pose references, nested shapes, texture slots and the
physical-3000/logical-5800 binary container. Regression tests additionally cover
keys beginning on the following line, empty nodes, connection-ordered texture
slots, camera switchers, root-child creation order and signed-zero curve bounds.
[`LEGACY.md`](LEGACY.md) cites the reference functions and construction rules.

Formatting, clippy with warnings denied, workspace tests, doctests, wasm32,
release build, full comparison, modern order, bake and the public-text guard
all exit 0 on the final candidate. Earlier failed receipts remain retained
outside the repository; focused tests were followed by a complete passing gate.
Fuzz seeds are reproducible; a new fuzz campaign remains a separate release step.

## Production-size files, whole streams

The corpus files are small. Three production-size inputs were compared against
the oracle over the whole canonical stream, every section, including `evaluated`
(a full scene evaluation at every sample time) and `order`: the generated
`grid400` benchmark (15 MiB) and the two Bistro interiors (40 and 46 MiB), plus
a small file with a `ByControlPoint` layer. `Surveyor.fbx` (355 MiB, 7.5 million
DOM nodes) could not be, see below. The streams are 1.3 to 40 million records, tens of GiB as
text, so `fbx-parity dump-scene --max-output 0` lifts the dumper's 8 GiB output
cap and `reference/compare_large.py` pipes each dump through `fbx-parity sections
--save`, which validates the stream in one pass (framing, checksums, identities)
and keeps one file per section. Two sections are equal exactly when their
record count and SHA-256 agree, which compares them record for record, in order,
bit for bit, with no tolerance. Equal sections are deleted, a differing section
is classified or printed.

| File | Records (oracle, Rust) | Oracle, Rust (s) | Sections equal | Differing sections |
| --- | ---: | ---: | --- | --- |
| `grid2-bycontrolpoint.fbx` | 2,177 and 2,173 | 0.0 | 15 of 17 | `globals`, `dom` |
| `grid400.fbx` | 1,279,040 and 1,279,036 | 15.1, 15.1 | 15 of 17 | `globals`, `dom` |
| `BistroInterior.fbx` | 32,755,827 and 32,755,827 | 104.7, 114.3 | 16 of 17 | `dom` |
| `BistroInterior_Wine.fbx` | 40,539,927 and 40,539,923 | 144.5, 159.5 | 15 of 17 | `globals`, `dom` |
| `Surveyor.fbx` | not compared | oracle refuses after about 8,000 | none | see below |

`Surveyor.fbx` could not be compared in full. The pinned oracle takes 2 h 14 min
of CPU on it (no output for the first 1 h 50 min), writes 5.5 GB of the `dom`
section and then stops with `canonical oracle: serialization/evaluation work
limit`: `reference/canon.c:31` caps the serialization work at 50,000,000 units
and the file has 7.5 million DOM nodes and 14.8 million values. The silent first
phase is consistent with the quadratic primary-element search in
`index_elements` (canon.c:216 to 223), which was not profiled further. A full
comparison needs a scratch build of the oracle with that cap raised
(`reference/build.py` on a copy of `canon.c`) and a run of
`compare_large.py --sequential`; the Rust side needs no change (its dump is
streamed and uncapped). Until then the 355 MiB file has no stream comparison;
its load is measured (time and memory) and nothing more.

The differing sections are the two classes the script names, never an
unexplained value:

- `dom`: ufbx modifies retained DOM arrays in place while loading, and the
  oracle dumps the modified array; fbx-rs keeps the file's content. Every
  differing record is a numeric array block of identical path and length: 3,029
  `Normals` arrays (Bistro), 3,223 `Normals` and one `UVIndex` (Wine, the
  array whose 1,188 out-of-range indices ufbx clamps), 118 `Vertices` (grid400).
  The corpus has the same class (271 of its 316 files, 0 other).
  The per-array proof (the oracle's array equals the oracle's own converted mesh
  data) is not repeated here; the `meshes` section is equal in every file.
- `globals`: ufbx records warnings (`Clamped index` x 1,188 on one mesh of the
  Wine file, `Ignoring geometry 'Normals' with bad mapping mode
  'ByControlPoint'` on the grid files) and fbx-rs does not reproduce the warning
  list; and `settings.axes` of the grid files, which are Z-up: ufbx leaves the
  file's axes there after converting to Y-up, fbx-rs records the axes of the
  loaded scene (`SceneSettings::axes`, documented as the target axes).

The comparison first failed on an ordering difference in the canonical dump, not
in the public lists (those were already ufbx's, 316 of 316 in `order`):
`curve_samples` listed curves in the public (creation) order, and `evaluated`
listed nodes, skin clusters and morph channels in it too. `reference/canon.c`
walks every element kind in canonical element order (`ordered_elements`, sorted
by `element_order`, canon.c:184 and :301; used at :677 and :633 to :685). The
Rust dump now does the same (`src/animation_canon.rs`). The 316 corpus files did
not show it: re-run over them with the section comparison, 45 are equal in every
section and 271 differ only in `dom`.

Memory was remeasured with the same scratch counting allocator on the original
and changed release builds. Values below are **phase-local peak live requested
heap bytes**, in MiB (2^20 bytes), including the input buffer held by the caller.
Each phase resets the high-water mark to the currently live heap; retained data
from earlier phases therefore remains included. Reallocation counts the change
in requested size; allocator-internal transient copies, metadata, fragmentation,
stacks and code pages are excluded. These are heap measurements, not RSS or
performance timings. The three inputs were read in place and hash-checked on
both runs; no external files were opened by either importer.

| Phase peak, MiB (before → after) | Surveyor | BistroInterior | BistroInterior_Wine |
| --- | ---: | ---: | ---: |
| Parse | 1,411.1 → 610.9 | 236.2 → 229.5 | 260.8 → 255.5 |
| Element construction | 2,201.3 → 1,020.4 | 241.4 → 231.5 | 266.6 → 257.8 |
| Graph and transforms | 3,071.8 → 1,093.3 | 252.9 → 243.6 | 280.7 → 272.3 |
| Meshes | 3,141.5 → 1,087.5 | 442.0 → 432.3 | 494.3 → 485.6 |
| Deformers | 3,308.4 → 1,217.0 | 438.7 → 429.1 | 490.6 → 482.1 |
| Materials and attributes | 3,192.6 → 1,107.9 | 443.2 → 430.9 | 495.6 → 483.9 |
| Animation | 3,955.2 → 1,408.0 | 499.6 → 463.4 | 559.8 → 523.9 |
| Owned scene publication | 3,272.7 → 1,441.5 | 465.5 → 450.6 | 517.9 → 508.0 |

| Input | Input MiB | Overall peak MiB (before → after) | Peak/input (before → after) | ufbx peak MiB | After/ufbx |
| --- | ---: | ---: | ---: | ---: | ---: |
| `Surveyor.fbx` | 355.3 | 3,955.2 → 1,441.5 | 11.13x → 4.06x | 1,014.3 | 1.42x |
| `BistroInterior.fbx` | 39.9 | 499.6 → 463.4 | 12.53x → 11.62x | 236.8 | 1.96x |
| `BistroInterior_Wine.fbx` | 46.5 | 559.8 → 523.9 | 12.04x → 11.27x | 261.1 | 2.01x |

All six original/changed default Rust loads and all three fresh pinned-ufbx
loads exited 0. All three changed loads also succeed with an explicit
`max_allocation = 1536 << 20` (1,536 MiB) internal buffer envelope. The ufbx
allocator callbacks count temporary and result allocations with the same
conversion options and `retain_dom = false`, plus the caller's input buffer.
Surveyor meets the 1,536 MiB heap target; the optional 1.1x-ufbx target remains
unmet. Its full canonical-stream comparison remains unverified as described
above; these memory results do not establish that comparison.

The loader now stores node states and adjustments sparsely, permutes elements in
place, reserves authored element and connection degrees exactly, and shares the
compact DOM child index. The binary lean parser validates curve/value subtrees
and then retains borrowed source ranges, restoring one object at a time. It also
packs binary connection records without retaining their syntax properties.
Original source ordinals, aggregate node/property/array/decompression limits and
compression validation are preserved; custom curve children keep the complete
subtree. The owned scene releases the DOM after publication.

`Scene::element_count()` returns the exact final canonical element total in O(1)
with no allocation or second parse. It requires a completed load and includes
root, opaque/unsupported, animation and generated helper/blend-shape elements,
rather than summing public typed lists. The three totals are 887,693, 2,908 and
3,457 respectively, equal to the pinned ufbx scene totals. Authored opaque kinds,
scale helpers and legacy synthesis are also covered by regression fixtures.

`Limits::max_allocation` now spans parsing, conversion and publication. Checked
reservations charge retained capacity plus old and new buffers during growth;
owned scratch, restored objects, indices and copied property lists refund their
charges after use. Some short-lived data remains conservatively charged until
load returns, so rejection may precede the actual heap peak. Caller input and
allocator/process overhead need a separate host allowance; the field is not an
RSS cap. Allocation-failure sweeps and a global-allocation-counter regression
cover accepted and rejected loads at several byte limits.

The final validation passed formatting, both clippy configurations with warnings
as errors, workspace tests, doctests, rustdoc, wasm32, the release build and the
public-text guard. Ordered modern output is unchanged on 316/316 files; the full
comparison passes 340/340 files (including 24 legacy inputs), and all 48 baked
streams match. Probe sources, binaries, input hashes, raw phase bytes and each
run's exit status are retained outside the repository.

### Curtains converted-space regression

Both additional Curtains inputs were compared over all 17 canonical sections,
including every mesh, texture, triangle, evaluated sample and typed-list order.
The final candidate streams were validated and compared with retained complete
oracle section counts and SHA-256s, with the input SHA-256 checked again.

| Input | Records on each side | Exact sections | Remaining documented differences |
| --- | ---: | ---: | --- |
| NewSponza Curtains YUp | 8,209,572 | 16 of 17 | Retained DOM array mutation |
| NewSponza Curtains ZUp | 8,208,059 | 15 of 17 | Retained DOM array mutation and reported file axes |

Every remaining differing section is also identical to the candidate section
previously classified by the full-stream comparator. No new difference class,
numeric tolerance or reordered hash was admitted. Material and texture records
are exact, including procedural shader inputs and file-texture links. Each file
has 2,001,184 exact triangle records. The original corpus remains 316 of 316
value comparisons and 316 of 316 order comparisons; bake remains 48 of 48.
The generated tests cover converted diagonal rounding, reversed winding,
Arnold/OSL semantic bindings, procedural file links and stored geometric
properties on an animated node. No asset bytes are embedded in these tests.

## Robustness

Four libFuzzer targets run on malformed input with small limits:
`binary` and `ascii` (the parser), `scene_dom` (parse and full canonical
conversion) and `load` (the public API, then a range check of every index
the returned `Scene` exposes, and evaluation of every animation stack,
binding and curve). The targets are built with debug assertions and overflow
checks. The release rule: each target starts from the corpus files as seeds
and runs in 600-second chunks over one persistent corpus, until a whole
chunk adds no edge coverage, with at least 600 and at most 7,200 CPU-seconds
per target; any crash, timeout (5 s) or memory limit (512 MiB) fails the run.

The `load` and `scene_dom` limits are sized so that all 316 supported corpus
files load and convert: `max_nodes` 2,048, `max_properties` 8,192,
`max_array_elements` 65,536, `max_decompressed_bytes` 262,144,
`max_objects` 1,024, `max_connections` 2,048, `max_allocation` 1 MiB. With the
earlier, smaller limits only 11 of the 340 seeds got past the parser, none of
them with a skin, blend shape or curve, and the deformer and animation
converters stayed almost unreached by seeds. Line coverage of the 340 seeds
alone, measured with `cargo fuzz coverage`: `scene_deform.rs` 0.97 to 73.2 %,
`scene_animation.rs` 22.6 to 75.9 %, `animation.rs` 19.2 to 48.6 %. After the
fuzz run, the final `load` corpus reaches `scene_deform.rs` 78.2 %,
`scene_animation.rs` 84.3 % and `animation.rs` 52.0 % of lines.

Pre-release run on the animation branch (340 corpus seeds plus the corpus of
the earlier run, one libFuzzer process per target; initial coverage is the
seeds alone):

| Target | CPU-minutes | Executions | Edge coverage, initial to final | Crashes | Stop | Peak RSS |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| `binary` | 99.3 | 228.8 M | 417 to 881 | 0 | chunk 10 added no coverage | 52 MiB |
| `ascii` | 49.4 | 102.9 M | 417 to 863 | 0 | chunk 5 added no coverage | 49 MiB |
| `scene_dom` | 122.1 | 3.2 M | 7,348 to 12,097 | 0 | chunk 14 added no coverage, at the cap | 136 MiB |
| `load` | 120.2 | 48.6 M | 5,729 to 10,421 | 0 | chunk 14 added no coverage, at the cap | 146 MiB |

CPU-seconds include corpus loading, so the last chunk of `scene_dom` and
`load` ends slightly past 7,200. `scene_dom` had one libFuzzer timeout: a
7.6 KB mutated Maya file that costs 0.42 to 0.50 s alone (it spends the
`max_animation_work` budget in the canonical sampling), reported while the
process ran at a tenth of its usual speed on a loaded machine. It is not a
defect and the run was resumed. No crash, out-of-memory or memory-limit
failure occurred in any target.

Pre-release run for 0.1.2 (fallible load, material and texture API, linear
bake, list ordering, scale-helper bakes). The `load` target now also checks the
documented node order (a parent precedes its children, depth never decreases),
the material and texture API (index ranges, lookups against the ordered
iterators, sorted source properties) and bakes every stack (default and the
first three) under four option sets (defaults; raw and trimmed; rotation-aware
reduction with transform properties; one set derived from a hash of the input
that varies rates, segment counts, thresholds, passes and the key, allocation
and work caps), checking that baked node indices are in range and increasing.
Reach, measured with `cargo fuzz coverage` on the 343 seeds alone (the 340
corpus files plus the three files in `tests/fixtures`): `bake.rs` 89.2 % of
lines, `scene_material.rs` 76.8 %, `api.rs` 68.2 %, `scene_animation.rs`
76.8 %. Same rule and flags as above (`-rss_limit_mb=512`, `-malloc_limit_mb=512`,
5 s timeout), four libFuzzer processes in parallel:

| Target | CPU-minutes | Executions | Edge coverage, initial to final | Crashes | Stop | Peak RSS |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| `binary` | 40.0 | 123.2 M | 438 to 891 | 0 | chunk 4 added no coverage | 52 MiB |
| `ascii` | 79.4 | 261.2 M | 438 to 902 | 0 | chunk 8 added no coverage | 52 MiB |
| `scene_dom` | 121.0 | 4.9 M | 8,512 to 13,420 | 0 | chunk 13 added no coverage, at the cap | 140 MiB |
| `load` | 120.5 | 27.0 M | 7,749 to 12,335 | 0 | chunk 14 added no coverage, at the cap | 151 MiB |

A first `load` run stopped after 50 CPU-seconds on an assertion in the new
harness code: it compared two `SourceProperty` values that hold a NaN with
`assert_eq!`. That is a harness error (NaN never equals itself), not a library
defect; the harness now compares names and map identity, the input was kept
aside as a harness false positive, and `load` was restarted from the same
corpus and run in full. No library crash, timeout, out-of-memory or
memory-limit failure occurred in any target.

Delta run for the four limits added after that run (`max_meshes`,
`max_layers`, `max_face_vertices`, `max_triangulation_work`), `load` target only.
The harness limits for the four are 16 meshes, 32 layers, 128 face vertices and
256 work units: every supported corpus file loads under them (measured: all load
with 8 meshes, 16 layers, 16 face vertices and 256 work units; 21 files are
refused at 8 layers, 8 at 8 face vertices and 2 at 64 work units), and a seed of a few
hundred bytes reaches each, one per limit (`limit-meshes.fbx`, `limit-layers.fbx`,
`limit-face-vertices.fbx`, `limit-triangulation-work.fbx`, each refused with
its own named `Limit`). Start: the 343 corpus seeds, the 4 limit seeds, the
earlier `load` corpus. Same flags as above (`-rss_limit_mb=512`,
`-malloc_limit_mb=512`, 5 s timeout, `-max_len=262144`), 600-second chunks, stop
at a chunk that adds no edge, with a 3,600 CPU-second cap:

| Target | CPU-minutes | Executions | Edge coverage, seeds to final | Crashes | Stop | Peak RSS |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| `load` | 60.0 | 17.8 M | 7,814 to 12,517 | 1 found and fixed, 0 after | cap reached; the last chunk still added 24 edges | 157 MiB |

The one crash, at 2,090 CPU-seconds, was a library defect, not a harness error:
the `load` harness asserts the documented node order (a parent precedes its
children, depth never decreases), and a node with no parent connection was not
treated as a child of the root. ufbx attaches such a node to the root
(`ufbxi_finalize_scene`, ufbx.c lines 18938 to 18943), so its children are one
level deeper than the node; fbx-rs gave them the node's own depth and
`Scene::nodes()` could list a child before a shallower node. The oracle confirms
it on a five-node file (`/graph/<child>/node_depth` 2, and the orphan listed in
`root.children`, against 1 and absent). Fixed in `project_graph`; regression
test `a_node_without_a_parent_connection_is_a_child_of_the_root`; the input is
kept as a seed and the run was resumed from the same corpus on the rebuilt
target (chunk 4 above ended in the crash, the other six chunks are clean).

Malformed-curve regressions
require errors for unsorted and nonfinite times, invalid attribute counts and
excessive key counts; duplicate times and finite repeat limits are also tested.

Deformer arrays are charged from their source counts before decoding,
including intermediate array storage. Adjacency is charged before building
it. Connection deduplication preserves first occurrence using an indexed
set; pose IDs, parent entries, depths and cluster lookups use index maps.
Traversal and sorting also consume `Limits::max_deformer_work` (default
268,435,456 units); excessive work returns an error. Regression tests cover
both byte orders of wide binary indices, allocation prechecks, duplicate
connections, a reversed pose chain, and multiple static meshes with a nested
shape. These stage budgets are not a process RSS limit.

Animation helper detection and key reconstruction preflight every curve's five
source array counts and charge their aggregate storage before decoding. Curve
ranges are cached once, independent of connection fan-out. Element-indexed maps
resolve curves, values, layers and helpers; sorted bindings support binary-search
evaluation with stable first-occurrence behavior. Each copied layer binding and
property name consumes the allocation budget, including vector growth and sort
scratch. `Limits::max_animation_work` bounds traversal and sorting in each
animation/helper stage (default 268,435,456 units).

Authored property extraction indexes unsupported value kinds once over the
stored property table; shared and overlapping caller-constructed ranges use
constant-time prefix queries. The sum of selected range lengths and the actual
indexed/string/comparison work each have a 67,108,864-unit cap per extraction.
String validation, flag scanning, and fallible in-place heap sorting charge byte
work before scanning, including long shared strings. Scratch index capacity and
returned property capacity share the allocation budget. The deterministic
property regression uses 1,000 and 8,000 shared ranges; its actual-work counter
excludes the separate conservative range-length admission charge.

All explicit fallible vector reservations use one exact-reservation helper.
It preflights requested storage, reserves the entire remaining accounting
envelope before invoking the allocator, and refunds unused capacity only after
checking the actual capacity and before use. Growth keeps old storage charged
while reserving new storage. Tests inject over-capacity reservations and verify
that the envelope is already charged inside the allocator callback. Capacities
beyond that envelope return an error before use. This bounds accepted Rust
capacities, not allocator-private rounding/metadata, allocations performed inside
the allocator, or process RSS; safe Rust does not control those internals.

ASCII special numbers follow ufbx 0.23.1's `ufbxi_parse_inf_nan` and token reader:
case-insensitive infinity and signed quiet NaN (including NaN payload spelling),
and legacy one-digit `d.#INF`, `d.#NAN`, `d.#IND` with optional trailing digits.
NaN sign bits are preserved. Bare special words keep integer zero; signed
numeric tokens use the normal float-to-integer conversion. Bare words of 64 or
more bytes keep the first-character numeric fallback, matching ufbx. Scalar,
retained-DOM, authored-property, and numeric-array consumers share these rules.
`1.#QNAN`, `1.#SNAN` and arbitrary suffixes such as `1.#BANANA` return errors,
as in the pinned oracle. These spellings were previously accepted as NaN.

The crate denies truncating, sign-losing and potentially wrapping casts. Numeric
storage bit patterns use explicit byte conversions; arithmetic narrowing checks
its range. Two local floating-point casts have range proofs. Deterministic scaling
regressions build deformer, pose, curve, layer and helper graphs at 32 and 256
elements and bound charged work by the input ratio and sorting factor. These
counters are conservative operation budgets, not wall-clock or RSS measurements.

## Deliberate differences from ufbx

- Resource admission for caller-constructed property DOMs rejects excessive
  aggregate range lengths, string/comparison work, or shared output/index
  capacity with `LimitExceeded`. Valid values within these budgets retain their
  ordering and last-duplicate semantics. The capacity accounting above applies
  before use; allocator-internal storage is outside this contract.
- Extrapolation types use the full signed 32-bit switch, as in ufbx's
  `ufbxi_read_extrapolation`: unknown representable values select constant
  extrapolation. Types or repetition counts outside that range return an error
  rather than wrapping into a valid setting. Negative representable repetition
  counts normalize to unlimited. Animation-connection cycles, including
  disconnected cycles and self-links, return errors instead of being ignored.
- Scalar mesh metadata outside signed 32-bit range is treated as absent rather
  than wrapping. Mesh source array and material counts above the signed 32-bit
  index domain return errors before decoding or deriving indices. Stored index
  sentinels and class identifier words retain their original bit patterns.
- An integer outside the signed 32-bit index range becomes an invalid index
  sentinel and sparse deformer conversion returns an error. ufbx 0.23.1's
  binary array conversion casts a 64-bit integer to 32 bits, so an invalid
  value such as 4,294,967,296 can alias vertex zero. Representable indices
  retain their values. Invalid/missing pose IDs and malformed deformer
  counts also return errors rather than silently dropping data.
- Blender `FullWeights` follow the same stable permutation as their shape's
  offsets. Source indices `[2, 0]` and weights `[20, 80]` therefore produce
  indices `[0, 2]` and weights `[0.8, 0.2]`. ufbx 0.23.1 sorts the offsets in
  `ufbxi_read_shape` but later copies these weights in source order in
  `ufbxi_finalize_scene`, yielding `[0.2, 0.8]` for this case. Preserving the
  weight-to-offset association is intentional; already sorted Blender
  shapes and the seven corpus rig files retain bit-exact stored deformers.

The earlier stored-rig comparison checked element names, IDs, properties,
flags and connections as well as deformer payloads, but excluded animation
property counts and the ANIMATED flag on animated inputs. The current full
stream comparison has no such element-metadata exemptions; it compares those
fields along with authored and evaluated animation.

`scene_dom` and `load` went flat only at the 7,200 CPU-second cap, after
coverage grew in every earlier chunk, so longer runs can still find new paths
in the converter.

## Not covered

- Legacy qualification is bounded to the listed inputs. The FBX 5.8 corpus
  has two cube variants; authored root/UV and binary-container fixtures extend
  those paths. Pre-6000 camera, light, nested material and link variants are
  outside this qualified set.
- Mesh positions stay undeformed: evaluation returns transforms, channel
  weights and skin matrices. Applying skin influences and sparse offsets to
  the vertex arrays remains the caller's responsibility.
- Camera and light modes the corpus does not contain: only three files carry
  cameras or lights. The other aperture, gate-fit and aspect branches are
  ported but not exercised.
- Load options other than the defaults: they are rejected, not converted.
- Browser WebAssembly: wasm32 was checked under Wasmtime only.
- Inputs beyond the qualified set: equivalence is shown on the 340 corpus
  files, seven legacy fixtures and the separately listed regressions, not
  proved in general.
- Performance is measured separately and is not part of equivalence.


## Converted-space triangulation

Public triangle indices and the canonical triangle stream use the loaded mesh's
converted positions and polygon-corner order. ufbx 0.23.1
`ufbx_catch_triangulate_face` reads `mesh->vertex_position` (ufbx.c:32416-32488):
quads choose the shorter diagonal, ties choose the first split, and concavity
can override that choice. N-gon ear selection uses the same loaded positions.
Triangulating source positions and then remapping reversed corners can change
both rounding decisions and triangle order. The generated near-square test
checks a centimetre-conversion tie and both winding directions; no asset bytes
are embedded in the test. Canonical emission borrows source topology without
rebuilding a second whole-scene draft; its temporary allocations remain bounded.

## Public material and bake additions

The public material projection retains all 23 computed feature flags with their
explicitness bits, effective authored/template properties in byte-name order,
and all texture attachments. Authored values override template defaults. String
and blob values and attachment property names retain source bytes. Layered
textures expose child texture indices, blend modes and alpha, plus the flattened
unique file-texture list. The canonical schema already covers these fields;
`check-public` also compares their public projection against canonical records.

PBR mappings cover FBX Lambert/Phong and the unknown-shader Phong fallback,
plus class-ID-identified 3ds Max Physical, PBR Metal/Rough, PBR Spec/Gloss,
glTF and OpenPBR materials. These include property prefixes, texture suffixes,
texture enable switches, default/explicit features and roughness/glossiness
remapping. Arnold and OSL standard surface shaders resolve semantic bindings:
`RenderAPI` selects the mapping table, the first binding table containing a
semantic supplies all its entries in stable order, and an absent semantic uses
its own property name. Values and textures are fetched independently, so a
reference-only texture property does not erase a preceding numeric value.
These rules follow ufbx 0.23.1 `ufbxi_read_binding_table` (ufbx.c:14701-14742),
`ufbxi_fetch_mapping_maps` (19968-20129), `ufbxi_finalize_scene`
(22303-22330, 22523-22542), and `ufbx_find_shader_prop_bindings_len`
(31442-31470). Arnold procedural shader textures retain their inputs and recursively flattened
file-texture links (`ufbxi_finalize_shader_texture`, ufbx.c:20560-20684).
OSL/select-output texture nodes, ShaderFX/Stingray bindings and Blender-specific
Phong mapping remain unported. Display-layer visibility, frozen state, colour
and node links follow `ufbxi_update_display_layer` (23393-23398). Stored geometric
transform properties are reset after baking, as in `ufbxi_modify_geometry`
(21313-21329).

`MaterialMap` remains exhaustive for 0.1.x compatibility. Making it
non-exhaustive is planned for 0.2.0; callers can currently construct and
exhaustively destructure it. The newly added material record types are
non-exhaustive.

Baking follows the pinned 0.23.1 key-time generation and independent channel
reduction: authored times, nonlinear segment samples, default stepped-key
handling, quaternion antipodal correction, and per-channel reduction passes.
Positive start trimming uses rounded FBX ticks. Playback bounds and duration
retain the original stack range, as in the pinned implementation. The aggregate
candidate-key cap is charged before adding times, including duplicate times and
keys removed by reduction; it is deliberately more conservative than an output
key count. Vector capacities are precharged and reserved fallibly. Numeric
property evaluation borrows lookup keys and avoids copying retained string/blob
values. Node sampling caches each node's static transform inputs once per bake and
samples only the inputs needed by each channel, including pivots for translation
and inherited scaling dependencies. Unrelated properties are sampled only for
their own baked channel. RotationOrder is resolved once per sample and reused
across rotation layers. Evaluation work charges static input lookups, node and
dependency visits, each evaluated property, each layer visited for that property,
connection traversal and animated layer weights. The work limit is therefore
stricter than the former node-visit count.

The bake API implements a subset of ufbx options. Maximum sample-rate culling,
custom step handling, transform overrides and configurable evaluate flags are
not exposed.

Scale helpers follow ufbx's rule (`ufbxi_bake_node_imp`). A scale helper node
carries the animated scale of the node it was generated for, and the nodes that
do not inherit that scale depend on its baked keys. When such a helper has
been baked, the dependent channel is resampled at the helper's baked key times
(a step key moves to its neighbour's time), its own animated keys are resampled
when the helper's scale is not constant, and its values are multiplied by the
helper's baked scale evaluated linearly between baked keys, as ufbx does, not by
the analytic scale. Nodes are baked in node order, so a helper precedes the
nodes that read it; baking a scale helper also bakes every sibling that has no
animation of its own, and a sibling's own scale helper when the helper of the
node whose scale that sibling inherits is scheduled. Such an unanimated sibling
has translation keys from the helper only and the stack bounds as rotation
and scale keys. This is the rule that was missing for `joint4` of
`maya_scale_no_inherit_7700_ascii.fbx`, whose omission also changed the key-time
extents. Baked properties are ordered by object, then by property name.
Spherical rotation reduction and floating-point arithmetic require exact oracle
comparison before claiming interchangeability on a file. The source and reference
probes are `examples/bake_dump.rs` and `reference/bake_probe.c`; they emit all times,
values, flags, constant bits and playback/key bounds as exact IEEE-754 bit patterns.

## Allocation refusal boundary

`load_with_limits` uses checked fallible reservations throughout syntax parsing,
graph construction, DOM conversion, transforms, geometry, deformers, materials,
animation indexing and public ownership conversion. Evaluation and bake buffers
follow the same rule. Allocator refusal returns `Error::Allocation`; it is never
turned into a partially converted mesh warning. Allocation failures in parser,
property and index errors preserve that type without constructing a diagnostic.
Other allocated diagnostics use a fallible formatter.

Ordered property/connection indexes use a private balanced tree in a fallible
vector arena. Lookup is logarithmic; sorted iteration uses parent
indices without scratch allocations. Stable conversion sorts reserve decorated
ordinal buffers before sorting, retaining the original tie order. DOM ancestor
state uses a fixed-size stack. Array decoding, lossy text, paths and embedded
content reserve storage before copying or extending it.

A thread-local test seam refuses the Nth reservation at the common allocator
callback. Public projection and bake tests sweep that seam. Integration tests
also use a real failing global allocator: they refuse the Nth allocation and
all later allocations for combined ASCII and compressed binary scenes, pose
and property evaluation, baking and invalid-input diagnostics. The fixtures
include mesh UV/color layers, materials/templates, layered textures, embedded
video content, skinning, blend shapes, a bind pose and an animated scale-helper
hierarchy. Every refusal must return the typed allocation error, with the
allocator still refusing during failure construction and cleanup. The separate
20,001-node regression refuses the public node-array layout from the reported
failure scenario.

The contract covers the default public load/evaluation/bake paths. The unstable
canonical dump writer's own value trees and output buffers, caller-invoked
`Clone`/`Debug`/`Display`, test fixtures and allocator internals are outside it.
Public loading shares one conservative buffer-capacity envelope across parsing,
conversion and publication, retaining old buffers during growth and refunding
owned scratch after deallocation. Evaluation and baking keep their separate
stage limits, and traversal work limits remain per stage. None is a process-RSS
bound: caller input, allocator-private metadata and fragmentation are excluded.
The live-allocation regression measures successful and refused loads under
several byte envelopes, covering ASCII, compressed binary, legacy deformation
and animation, and a larger node hierarchy; every measured heap peak fits the
configured load allowance. Executable tests establish the
covered fixtures and allocation points; the full path guarantee also depends on
the source audit of the remaining pre-reserved pushes, copies and error paths.

### Mesh refusal and bake-sort regressions

The default load/evaluation/bake allocation audit covers stable `sort`/`sort_by*`,
`to_vec`, iterator `collect` into vectors, vector clones and allocated formatting.
The remaining unbudgeted stable sort was the final baked-property sort. It now
carries the insertion ordinal beside each creation rank and sorts in place by
both, preserving equal-rank property order without scratch allocation. Existing
conversion stable sorts use checked ordinal buffers. Other matches are budget
helpers, borrowed ranges and fixed records, tests, or canonical dump value-tree
and output construction outside the default API allocation contract. No further
unbudgeted copies or formatting were found on default load/evaluation/bake paths.

Both typed and canonical mesh-conversion warning branches now propagate
`Allocation`, `ResourceLimit` and `LimitExceeded` unchanged. Their warning storage
and formatting use the fallible budget helpers. The material warning producers
already do so and do not catch resource errors. Invalid geometry remains a
warning with an empty public mesh, including vertex arrays whose length is not
divisible by three; resource refusals cannot use that fallback.

`out_of_creation_order_bake_sort_allocator_refusals_are_typed` bakes 3,072
properties across nodes whose hierarchy order differs from creation order,
checks creation order and all property ties, then refuses the final eight real
allocator requests, including the final property publication allocation.
`invalid_vertex_count_loads_with_a_warning` retains the malformed-geometry
compatibility boundary. `decreasing_limit_envelopes_never_substitute_empty_meshes`
halves every load limit independently through zero, checks typed refusals and
compares accepted mesh attributes, triangles and warnings with the default
load. It always covers repository fixtures; setting `FBX_LIMIT_CORPUS_PATHS` to
a newline-delimited input manifest adds the full external corpus.

Verification (2026-11-01): the decreasing-envelope test passed 138,155 loads
across all 340 corpus files and 10 repository fixtures. The six real-allocator
regressions and five limit/geometry tests pass. Formatting, default/all-feature
clippy with warnings denied, workspace tests, doctests, rustdoc with warnings
denied, wasm32, release builds, comparator fault tests and the public-text guard
all exited 0. Modern order is unchanged on 316/316 files; full comparison is
340/340 equal, including all 24 legacy files; baking is exact on all 48 streams.
The initial focused run exited 101 because the new authored bake fixture lacked
required curve metadata; after correcting that fixture, both the focused rerun
and the full gate passed.

### Legacy seed additions

`python3 fuzz/make_legacy_seeds.py OUTPUT` emits seven self-authored seeds:
legacy takes and Properties60, named skins/poses/nested shapes, the 5.8
root/UV layout, material texture layers, a 3000 container declaring FBX 5800,
and little- and big-endian binary files with compressed geometry and
heterogeneous keys.
The same authored files are regression fixtures. Add these to each target's
persistent corpus before the separate pre-release fuzz campaign; no new fuzz
campaign is claimed by this change.

### Legacy pre-release fuzz run (0.1.3)

Targets `binary`, `ascii`, `scene_dom` and `load`, built with debug assertions
and overflow checks (`cargo +nightly fuzz build -s none -a -O`). Start: 41 seed
files: the 24 legacy FBX 5.8/6.x corpus files (ASCII, little- and big-endian
binary, Blender 2.79, Maya and 3ds Max exports), the 7 authored legacy seeds
from `fuzz/make_legacy_seeds.py` and the 10 files in `tests/fixtures`. No
earlier corpus was reused. Same rule and flags as the earlier runs
(`-max_len=262144`, `-timeout=5`, `-rss_limit_mb=512`, `-malloc_limit_mb=512`,
600-second chunks over one persistent corpus, stop at a chunk that adds no edge
after at least 600 CPU-seconds, cap 7,200 CPU-seconds), four libFuzzer
processes in parallel.

Reach, `cargo fuzz coverage` on the `load` target with the 41 seeds alone
(line coverage): `legacy.rs` 88.9 %, `syntax.rs` 81.3 %, `syntax/ascii.rs`
70.9 %, `scene.rs` 85.6 %, `scene_animation.rs` 73.1 %, `scene_deform.rs`
72.3 %, `scene_material.rs` 59.2 %, `scene_geometry.rs` 49.9 %, `bake.rs`
91.3 %, whole crate 57.0 %. The public face triangulation over converted
(Y-up) positions, `triangulate_face_into_positions`, ran 142 times from the
Z-up Blender seeds through `Mesh::triangles`; the `load` harness did not
change.

| Target | CPU-minutes | Executions | Edge coverage, seeds to final | Crashes | Stop | Peak RSS |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| `binary` | 120.0 | 1,417.8 M | 376 to 1,022 | 0 | cap reached | 59 MiB |
| `ascii` | 50.0 | 591.9 M | 376 to 1,045 | 0 | chunk 5 added no coverage | 57 MiB |
| `scene_dom` | 121.9 | 3.5 M | 9,027 to 15,185 | 0 | cap reached | 132 MiB |
| `load` | 120.4 | 20.2 M | 8,056 to 15,029 | 0 | cap reached | 204 MiB |

Every chunk of every target exited 0; no crash, timeout, out-of-memory or
memory-limit failure and no artifact file. `ascii` met the flat-chunk rule
(chunk 5 added none). `binary`, `scene_dom` and `load` ran to the 7,200
CPU-second cap: their last full 600-second chunks still added 1, 44 and 37 edges,
and the short final chunk to the cap added none, so the cap, not a flat 10
minutes, ended them.
