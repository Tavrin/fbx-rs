# Load speed result

**SHIPS under the instruction, per-file, RSS and correctness gates.**
Total instructions fall 27.37%; all 316 files improve. The separate
1.5× ufbx target is not met: the result is 1.890× ufbx 0.23.1.

[Attribution](ATTRIBUTION.md) records the baseline investigation. Property
extraction was the largest hotspot. One bounded pass now identifies the first
property with a BinaryData child, preserving error order and parent scope;
there is no added allocation. Numeric mesh words now use checked 1/4/8-byte
conversions rather than per-byte accumulation. Endianness, sign, negative zero,
NaN bits, index bounds and truncation behavior are retained. No API changes,
new dependencies, changed limits, floating-point reassociation or Git mutations.

Final paired measurement: 64 loads/frees per process, three samples per file,
rotating main/candidate/syntax/ufbx order; per-file medians summed. Same build
flags, options and cleanup as attribution. ufbx does not explicitly triangulate;
Rust's public Scene does. Peak RSS is the median of three process high-water
marks, not divided by the repetition count. No wall-clock speed claim is made.

| Implementation | Corpus instructions (billions) | vs main | vs ufbx |
| --- | ---: | ---: | ---: |
| main 0.1.1 | 2.005219 | 1.000 | 2.603 |
| candidate | 1.456365 | 0.726 | 1.890 |
| ufbx 0.23.1 | 0.770365 | 0.384 | 1.000 |
| syntax only | 0.661616 | — | — |

Worst file ratio: `bathtub.fbx`, 0.880214× main (limit 1.02).
All per-file counters, RSS medians, input hashes and executable hashes are in
[measurements.json](measurements.json). The table below uses the ten largest
inputs by byte size. Instruction columns are millions; RSS columns are KiB.

| File | Main instructions | Candidate instructions | vs main | vs ufbx | Main peak RSS | Candidate peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| showerRound.fbx | 21.423 | 16.043 | 0.749 | 2.217 | 4864 | 4848 |
| washerDryerStacked.fbx | 19.455 | 15.066 | 0.774 | 2.217 | 4932 | 4728 |
| shower.fbx | 16.216 | 12.414 | 0.766 | 2.160 | 4584 | 4544 |
| kitchenStove.fbx | 14.578 | 11.878 | 0.815 | 2.108 | 4364 | 4280 |
| washer.fbx | 14.392 | 11.440 | 0.795 | 2.140 | 4632 | 4172 |
| blender_279_sausage_7400_binary.fbx | 51.503 | 25.335 | 0.492 | 4.239 | 4920 | 4804 |
| chairDesk.fbx | 11.211 | 9.237 | 0.824 | 2.032 | 4152 | 4096 |
| bathtub.fbx | 9.767 | 8.597 | 0.880 | 1.994 | 3912 | 3784 |
| bedBunk.fbx | 17.756 | 10.891 | 0.613 | 2.270 | 4460 | 4440 |
| CommonTree_Snow_2.fbx | 13.149 | 10.358 | 0.788 | 2.103 | 4424 | 4412 |

Maximum of those ten RSS medians: main 4.816 MiB,
candidate 4.734 MiB. All ten are lower.

Verification (all exit 0):

- Full bit-exact corpus classification: 316 equal, 0 deferred, 0 unexplained;
  24 older files rejected; 460 independently proved retained-DOM mutation arrays.
- Public API comparison: 316 files, 3,664,695 values, no mismatches.
- Workspace and default-feature library tests; both clippy configurations with
  `-D warnings`; fmt; public-text; default-feature wasm32 library build.
- Scaling: 8× inputs cost 7.95–9.72× charged work across the five graph cases.
- Wasmtime: exactly three fixtures, canonical output identical to native and
  CLI; public Scene and evaluated poses identical to native. An initial empty
  fixture-list receipt was rejected; the corrected check requires three.
- New regressions check property-error order and BinaryData parent scope, plus
  numeric words in both byte orders, signed extrema, float bits and truncation.

The baseline and candidate executables, raw stat/RSS samples, both profile
attempts, source bindings, build logs and gate receipts are retained externally.
The fresh temporary Cargo target is deleted after verification. No publish or
integration action is included.
