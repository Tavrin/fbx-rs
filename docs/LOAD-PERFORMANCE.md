# Load performance

The load optimizations preserve the parsed values and public scene evaluation.
Decoded property lists are cached within a document, and animation defaults
are shared by template while authored properties retain precedence. The cache
indexes all property owners once, avoiding repeated insertion into a growing
sorted table. Caching is optional when its allocation does not fit the existing
envelope; per-call property and allocation checks still apply.

Short ASCII integers and exactly rounded decimals use specialized parsing;
other tokens retain the general parser. Binary words use fixed-width reads,
compressed arrays share one resettable decompressor per document, and owned
collections fill their charged capacity before checking for growth.

## Measurement method

The comparison uses 316 fixed, hash-verified inputs. Each instruction sample
loads every input four times, subtracts the same driver's input-reading floor,
and divides by four. Results are medians of three samples with rotating driver
order. Both Rust drivers use the same release configuration and frame pointers.
The before executable predates these edits; it is the retained 0.1.4 baseline,
not a measurement of a later upstream revision.

The reference loader is version 0.23.1 with the pinned converted-space options,
external file access disabled, and retained DOM enabled. A second reference
measurement disables retained DOM and is reported separately. The Rust load
driver includes public scene ownership and triangulation. The syntax driver
only parses; subtracting it from load estimates the remaining conversion cost
but does not isolate individual conversion stages.

Stage attribution uses `perf record -e ibs_op//` with frame-pointer call stacks.
It classifies user samples by the first recognized stage in the stack, with
allocator leaves counted separately. These are sampled execution shares, not
per-function instruction counters. Throttled captures are excluded.

## Results

Instructions are billions per complete corpus load:

| Work | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Syntax parse | 0.699644 | 0.607563 | 13.2% |
| Load minus syntax | 0.807808 | 0.544279 | 32.6% |
| Public load | 1.507452 | 1.151842 | 23.6% |

The retained-DOM reference uses 0.767783 billion instructions, and the
reference without retained DOM uses 0.645265 billion. The public load ratio
therefore improves from 1.963 to 1.500 against the retained-DOM reference;
the final ratio against the reference without retained DOM is 1.785.
The 1.2 ratio target is **not met**: it requires at most 0.921339 billion
instructions on this workload, about another 20% reduction from this result.
These counters do not establish a wall-clock or large-scene speedup.

The unthrottled IBS captures contain 1,707 baseline, 1,186 optimized and 689
reference user samples. Kernel samples are excluded. Broad stage shares are
reported below; their finite sample counts and inlining limit interpretation
of small differences. An increased share does not establish increased work.

| Stage | Before samples | After samples | Reference samples |
| --- | ---: | ---: | ---: |
| Inflate | 19.16% | 24.11% | 21.92% |
| ASCII parse | 16.17% | 14.08% | combined below |
| Binary parse | 6.03% | 6.07% | combined parse: 29.03% |
| Properties | 14.82% | 8.60% | 3.34% |
| Animation | 12.60% | 9.19% | 0 sampled |
| Allocation | 6.21% | 7.50% | 1.89% |
| Graph | 7.21% | 8.94% | 36.14% |
| Meshes | 5.86% | 4.81% | 4.35% |
| Materials | 3.98% | 5.73% | 0 sampled |
| Array conversion | 3.16% | 2.53% | included in other stages |
| Triangulation | 3.10% | 5.48% | included in other stages |
| Public ownership conversion | 1.58% | 2.36% | included in other stages |
| Deformers | 0.12% | 0.17% | 0 sampled |
| DOM index | included in other stages | included in other stages | 3.34% |
| Other | 0 sampled | 0.42% | 0 sampled |

The largest remaining sampled costs are inflate and ASCII parsing. Property
and animation work shrink substantially in share alongside the measured
reduction in conversion instructions. Exact per-stage instruction counts are
not inferred from the IBS shares.

## Qualification

All final checks exit 0:

- `cargo fmt --all --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo clippy --locked -p fbx-rs --all-targets -- -D warnings`
- `cargo test --locked --workspace --all-features --no-fail-fast`: 199 passed,
  including the workspace doctests
- `cargo test --locked --doc -p fbx-rs`
- `RUSTDOCFLAGS=-Dwarnings cargo doc --locked -p fbx-rs --no-deps`
- `cargo build --locked -p fbx-rs --target wasm32-unknown-unknown`
- Release workspace and bake-driver builds
- Full-corpus decreasing allocation-limit envelopes
- `bash tools/check_public_text.sh`
- `python3 reference/test_corpus_comparison.py`
- Unchanged scene comparator: 340 equal, zero mismatches, including legacy
  inputs and the existing exact oracle-side DOM-mutation classifications
- Unchanged order comparator: 316 equal
- Unchanged bake comparator: 48 exact comparisons across 12 inputs
- Public scene cross-check, including sampled animation evaluation: 316 passed

Regression tests cover integer and decimal parsing boundaries, repeated
property reads and their limits, cached errors without output allocation,
shared template defaults with authored overrides, and independent compressed
streams including a later checksum failure. No fuzz campaign was run.

## Decisions

- Keep a document-local decoded-list cache rather than introduce borrowed
  references into the owned public scene. It adds bounded temporary storage;
  removing it is a local implementation change with no public API migration.
- Share animation template defaults rather than copy them for every object.
  Authored entries win on lookup. Reversing this requires restoring the merged
  per-object table and its former memory and loading cost; no API change is
  required.
- Keep the general numeric parser as fallback rather than accept approximate
  decimal conversion. Removing either fast path is local and preserves the
  existing syntax and value contract.
- Keep the existing decompression dependency rather than add a backend in
  this change. Reversing decompressor reuse is local; changing backends would
  require fresh malformed-stream and corpus qualification.
