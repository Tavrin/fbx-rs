# fbx-scene-canon/1

This is the canonical oracle format. It is UTF-8 JSON Lines with LF endings and fixed
member order. No JSON numeric literals are used. A successful process exit and
a valid footer are both required. A matching prefix is never a valid dump.

The receipt binds input bytes/length, the oracle profile, C/header SHA-256,
field-coverage SHA-256 and dumper source SHA-256. Source filename hashes are
receipt data and do not participate in scene equality. Build commands, compiler,
target, binary hashes and Rust binding archive verification are retained in
scratch `build-receipt.json`. Neither load durations nor addresses are emitted.

Sections, in order, are `options`, `globals`, `dom`, `elements`, `connections`,
`graph`, `meshes`, `skins`, `blend_shapes`, `materials`, `cameras_lights`,
`curves`, `advanced`, `triangles`, `curve_samples`, `evaluated`. Each starts with
`begin`, contains `field` records with a path and value, and ends with `section`
containing its field count and SHA-256 over the exact field lines. The footer
counts all fields and hashes every preceding byte, including the receipt and
section delimiters. Checksums are integrity checks, not numeric comparison rules.

Source object keys for 7.x use signed source ID and retained DOM preorder
ordinal. Legacy keys use type bytes, full source-name bytes and source ordinal.
The root has its own key. When a legacy Model/Take has multiple typed
representations, one primary element owns its source key and the others are
explicit generated attachments/layers. Generated elements use owner, role, type and generated
ordinal; process addresses and dense oracle IDs never identify an object.
Duplicate source IDs are diagnosed explicitly. Children, attributes, material
slots, faces/corners, layers, influences and keys retain their sequence order.
Raw connection kinds, endpoint properties and source order remain in the DOM;
the resolved connection table is a canonical lookup. The element table is ordered
by signed source ID then source ordinal, or by legacy type/name bytes/ordinal.

Leaves use `i64`/`u64` decimal strings, boolean literals, f32 eight-digit hex and
f64 sixteen-digit hex. Strings have byte length and `bytes_hex`; absent pointers
are null and empty sequences have an explicit zero length. Vectors, matrices and
quaternions contain every component. Sequence lengths are exact; contiguous
primitive arrays use chunks of 4096 elements. Structured arrays recurse by index.
Properties include type, flags, all value projections and the complete defaults
chain, with a bounded traversal. Authored/default origin remains distinct.

DOM arrays retain exposed width and exact bits. Pointer-backed string/blob arrays
are decoded into their byte strings. Blobs have byte length/SHA-256; raw DOM blob
chunks retain the local bytes for offset diagnosis. CreationTimeStamp is
nonsemantic DOM metadata; FileId is preserved exactly. Source filename bookkeeping and allocator statistics
are excluded from typed metadata. Actual texture/video filenames remain semantic.

The two generated `coverage-*.json` files classify reachable public header fields,
including aliases, implementation fields and unsupported advanced evaluation.
They are source manifests, not corpus manifests. Each normalized material map
and feature has its public field name; complete vector
union views preserve all components. Generated C size assertions check each
selected union representation against every alias. `options-*.json` pins every
initialized load option and its section hash. Zero sentinels select the defaults
of the separately hash-pinned C implementation. Both profiles retain DOM/media,
force FBX format, disable external loading, and install a deny-open callback.
The `engine-converted` options are those of a downstream engine's ufbx-based
loader (its source hash is pinned in `pins.json`): right-handed Y-up target
axes, 1 m units, geometry-modifying space conversion and compensated
inheritance. Allocator caps are 4 GiB and 2^24 allocations each; the runner separately
bounds each process address space to 3.5 GiB. A budget failure is a failed oracle
run and never a format rejection.

Parsed numbers, including curve time/value/min/max, defaults, authored Euler
angles and reconstructed f32 tangent handles, are exact. Triangulated corner
indices are a separate exact stream. No general epsilon applies. The retained
DOM does not expose original scalar numeric widths or bits discarded during
ufbx conversion/sanitization. The input hash binds that original evidence;
`NUMBER` integer/real projections are compared exactly as exposed. This tool
does not claim to recover information discarded by either pinned oracle.

Curve samples include every key, seconds-nextafter neighbours, source-tick
neighbours where KeyTime is exposed, segment quarters, take boundaries and
outside-take samples, and a 30 Hz grid from take start with the exact end once.
Each sample has exact f64 time bits and provenance (curve, key/segment index,
fraction or tick numerator/denominator). Composed scene samples deduplicate
identical time bits using deterministic provenance; individual curve samples
retain all requests. Evaluation is bounded separately: 200000 requested samples
per curve/take, 10000000 element-samples per take, 50000000 serialization work
steps per dump. A limit failure invalidates the whole dump.

Evaluation covers each layer/property and composed property, local/geometric
transforms, node/world/geometry matrices, cluster matrices and morph weights.
Only the comparator's closed evaluated path schema recognizes `eval_f64` and
`eval_quat`. Finite f64 components use absolute 1e-9 plus relative 1e-8;
quaternions use angular error at most 1e-7 radians with sign equivalence.
Nonfinite evaluated values fail; there are no admitted fixture exceptions.
All component counts, sample times, topology and other discrete fields stay
exact. No final f32 engine projection is emitted, so the proposed f32
projection tolerance is not enabled.

The comparator validates both entire streams, source pins, coverage, pinned
options, section order, counts and checksums before comparing values. It rejects
duplicate source identities, dangling references, duplicate JSON keys, rounded JSON numbers, malformed bits, undeclared tolerance
tags, trailing records and truncation. A semantic difference reports the input
hash, profile, object-containing path, component index and both exact values.
Profile mixing is rejected by default. Explicit override skips profile-specific
receipt/option equality while validating each against its own pins; remaining
scene differences still fail.

Advanced constraints/caches/NURBS/LOD/selection/display are preserved through
typed fields, identities, properties and DOM. Their evaluated semantics are not
compared. Repeat identity proves determinism of
this oracle on the admitted corpus, not correctness of a future Rust parser.
