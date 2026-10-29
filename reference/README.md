# Canonical FBX reference tools

This directory builds the ufbx oracles that fbx-rs is compared against. It is
not part of the published crate. ufbx is MIT OR Unlicense; its licence text is
in [LICENSE-ufbx](LICENSE-ufbx). The ufbx sources themselves are not vendored:
they are build inputs, verified against the SHA-256 values in `pins.json`.

Two oracle profiles exist:

- `historical-raw`: upstream ufbx C 0.11.0 (commit
  5d61cfce6272523f7a51b9d8580e156b352e1247) with default load options.
- `engine-converted`: ufbx C 0.23.1 as bundled by the Rust `ufbx` crate 0.11.5,
  with the load options of a downstream engine's ufbx-based loader (right-handed
  Y-up, 1 m units, geometry-modifying space conversion, compensated
  inheritance). This is the profile fbx-rs reproduces.

The tools load from memory and deny every external file-open request.

All inputs, corpus manifests, dumps and logs belong in a scratch directory
outside this repository. Never commit them. See [SCHEMA.md](SCHEMA.md) for
fields and numeric rules, [FIXTURES.md](FIXTURES.md) for fixture admission and
[DECISIONS.md](DECISIONS.md) for the oracle's design decisions. `scene_probe.c`
is an older inventory/timing probe, separate from the canonical dumper.

Requirements: a C11 compiler, the OpenSSL development library, Python 3 with
`pycparser==2.23`, and Rust. The build and qualification run CPU only and need
about 4 GB of memory. The scripts read these variables and stop with an error
when one is missing:

```sh
export CARGO_TARGET_DIR=<target-dir>
export SCRATCH_DIR=<scratch>
export FBX_HISTORICAL=<ufbx-0.11.0-source-dir>   # ufbx.c and ufbx.h at the pinned commit
export FBX_BINDING=<cargo-registry>/src/<index>/ufbx-0.11.5/ufbx
export FBX_CRATE=<cargo-registry>/cache/<index>/ufbx-0.11.5.crate
mkdir -p "$SCRATCH_DIR"
python3 -m venv "$SCRATCH_DIR/tools"
"$SCRATCH_DIR/tools/bin/pip" install pycparser==2.23
```

Build both dumpers and the comparator (source directories may be relocated;
their hashes must match). `--profile-source` optionally re-checks the
downstream loader source the profile options were read from against its
pinned hash.

```sh
ulimit -v 3670016
python3 reference/build.py --historical "$FBX_HISTORICAL" \
  --converted "$FBX_BINDING" --crate "$FBX_CRATE" \
  --python "$SCRATCH_DIR/tools/bin/python"
cargo build --locked -p fbx-parity
```

Run either dumper, then validate or compare complete dumps:

```sh
"$CARGO_TARGET_DIR/historical-raw/fbx-canon" input.fbx > "$SCRATCH_DIR/a.jsonl"
"$CARGO_TARGET_DIR/engine-converted/fbx-canon" input.fbx > "$SCRATCH_DIR/b.jsonl"
"$CARGO_TARGET_DIR/debug/fbx-parity" validate "$SCRATCH_DIR/a.jsonl"
"$CARGO_TARGET_DIR/debug/fbx-parity" compare "$SCRATCH_DIR/a.jsonl" "$SCRATCH_DIR/a.jsonl"
```

Comparison exits 0 for equality and 1 for a difference or invalid dump; stderr
gives the first path and exact values. Different profiles are refused. Use
`compare --allow-profile-mixing A B` only for explicit cross-profile investigation;
it still validates each profile's pins/options and reports scene differences.

`fbx-parity dump-scene input.fbx` writes the Rust converter's stream in the
same format under the `engine-converted` profile, and
`fbx-parity compare-scene ORACLE RUST` compares it with the oracle's.

Qualification: `qualify.py prepare` builds the public corpus manifest from a
corpus manifest and the pinned ufbx Git tree listing (it also adds five pinned
media fixtures and the self-authored ordering/colour regression). `qualify.py
run` then makes three dumps per path and profile plus real-domain corruption
tests. A run name must be new.

```sh
python3 reference/qualify.py prepare --manifest <corpus>/corpus-manifest.jsonl \
  --tree <corpus>/ufbx-v0.11.0-tree.json
ulimit -v 3670016
cargo test --locked
PYTHONDONTWRITEBYTECODE=1 python3 reference/test_harness.py
PYTHONDONTWRITEBYTECODE=1 python3 reference/qualify.py run --run qualification-001
python3 reference/hygiene.py --corpus-manifest "$SCRATCH_DIR/public-corpus.jsonl"
```

Field coverage and options are checked on rebuild; changing either requires
explicit `--write-coverage`, a reviewed field diff and a new qualification.

## Retained-DOM oracle and equivalence

`dom.c` and `build-dom.py` build a separate historical-raw retained-DOM oracle.
`dom_dump.py` binds its stream to input, oracle, runner and build-receipt
hashes and writes a checksummed JSONL footer. Do not commit corpus inputs or
dumps. ufbx's retained DOM discards source scalar type codes and converts some
array values, so it is not compared byte for byte with the Rust stream;
`dom_equivalence.py` projects the Rust stream onto ufbx's representation
(rules and ufbx line citations are in its docstring) and compares every value
bit for bit. See [VERIFICATION.md](../docs/VERIFICATION.md) for the counts.

```sh
cargo build --locked --release -p fbx-parity --example dump_syntax
python3 -I reference/dom_equivalence.py \
  --manifest "$SCRATCH_DIR/public-corpus.jsonl" \
  --oracle-dir "$SCRATCH_DIR/oracle-corpus" \
  --exe "$CARGO_TARGET_DIR/release/examples/dump_syntax" --out equivalence.json
```

It exits 0 only when no file is `failing`. `test_dom_equivalence.py` holds its
fault tests.

In `fbx-ufbx-dom/2`, node records preserve preorder and depth; value records
contain ufbx's DOM type, integer and f64 fields, string bytes, and blob bytes as
hex. Numeric arrays encode each element as a fixed-width big-endian hex word.
For raw-string arrays, `blob_hex` concatenates an eight-byte big-endian length
and the bytes for each element; `integer` is the element count. Earlier `/1`
streams are invalid because they included raw-string structure pointers.
`validate_dom.py` checks stream checksums, record counts, hierarchy, typed array
lengths, and raw-string-array framing. Supplying `--input` and
`--build-receipt` also checks those bindings. This validates the oracle stream,
not its equivalence to the Rust syntax DOM.
