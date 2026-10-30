# Load instruction attribution

Baseline: main `2a251fd99d0254a57da41882d74bf656d6164a8a` (0.1.1).
316 supported corpus files (166 binary, 150 ASCII); the ten largest are
selected by input byte size. Both implementations use the engine-converted
options, including retained DOM, external opens denied and normal generation.
Rust loads the public Scene, including triangulation. C loads/frees ufbx 0.23.1;
it does not explicitly triangulate. Thus the ufbx comparison includes that
additional Rust output cost. Source/header hashes match reference/pins.json.

`instructions:u`, median of three per file, 32 loads/frees per process,
rotating implementation order, then summed. File read and process startup
remain at 1/32 weight. Rust release uses line information and frame pointers;
C uses `-O2 -g -fno-omit-frame-pointer`. No fast math. Counters include cleanup.

Stage values below are **sampled instruction estimates**, in millions:
three instruction-event call-stack profiles, fixed requested period 100003,
normalized per file to its measured stat total, then median of three.
They are not independently counted stage deltas. Profiles are pinned to CPU 0;
there were zero lost samples. An earlier unpinned profile had out-of-order
mapping events and was retained but replaced for the table.
Shared property extraction is charged to
scene conversion; DOM build identifies retained-DOM/DOM-push stacks and their
allocation callees. Inlined DOM construction that cannot be separated from
parsing remains in parse. Other includes free, startup and unresolved stacks.
This attribution has sampling/skid/stack-unwind uncertainty, especially for
small stages and individual files; the shipping gate uses stat counters only.
Independent stage medians need not sum exactly to the stat total.

The largest Rust leaf hotspot is `scene_property::authored` (~29% of sampled
instructions): it scans every DOM node for BinaryData for each authored
property. Inflation (~10%) and triangulation (~5%) follow. This attribution was
identified before modifying the loader; the repaired pinned table uses the
unchanged baseline executables.

| Workload | Implementation | Parse | DOM build | Scene conversion | Meshes | Materials | Deformers | Animation | Other | Total (stat) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Corpus | Rust | 619.49 | 82.75 | 826.63 | 278.64 | 18.58 | 1.18 | 161.91 | 17.60 | 2008.24 |
| Corpus | ufbx | 518.59 | 122.12 | 97.27 | 23.83 | 0.54 | 0.10 | 0.24 | 10.22 | 772.98 |
| Binary | Rust | 334.58 | 34.34 | 422.83 | 249.73 | 10.20 | 0.90 | 85.36 | 9.06 | 1147.49 |
| Binary | ufbx | 280.94 | 62.73 | 54.07 | 19.05 | 0.24 | 0.07 | 0.20 | 5.01 | 422.53 |
| ASCII | Rust | 284.92 | 48.50 | 401.86 | 28.91 | 8.42 | 0.27 | 76.55 | 8.80 | 860.75 |
| ASCII | ufbx | 237.65 | 60.04 | 42.59 | 4.75 | 0.31 | 0.05 | 0.03 | 5.25 | 350.45 |
| showerRound.fbx | Rust | 10.47 | 0.37 | 8.55 | 0.77 | 0.11 | 0.00 | 1.23 | 0.06 | 21.43 |
| showerRound.fbx | ufbx | 6.08 | 0.55 | 0.42 | 0.15 | 0.00 | 0.00 | 0.00 | 0.03 | 7.24 |
| washerDryerStacked.fbx | Rust | 9.76 | 0.34 | 6.87 | 0.85 | 0.17 | 0.00 | 1.07 | 0.09 | 19.47 |
| washerDryerStacked.fbx | ufbx | 5.26 | 0.67 | 0.60 | 0.22 | 0.00 | 0.00 | 0.00 | 0.04 | 6.80 |
| shower.fbx | Rust | 8.47 | 0.67 | 4.15 | 0.74 | 0.07 | 0.00 | 1.06 | 0.06 | 16.22 |
| shower.fbx | ufbx | 4.72 | 0.48 | 0.36 | 0.13 | 0.00 | 0.00 | 0.00 | 0.04 | 5.76 |
| kitchenStove.fbx | Rust | 6.76 | 0.41 | 5.43 | 0.80 | 0.12 | 0.00 | 0.99 | 0.06 | 14.60 |
| kitchenStove.fbx | ufbx | 4.55 | 0.52 | 0.41 | 0.11 | 0.00 | 0.00 | 0.00 | 0.03 | 5.64 |
| washer.fbx | Rust | 7.12 | 0.31 | 4.99 | 0.58 | 0.17 | 0.00 | 0.88 | 0.06 | 14.40 |
| washer.fbx | ufbx | 4.24 | 0.51 | 0.46 | 0.13 | 0.00 | 0.00 | 0.00 | 0.04 | 5.35 |
| blender_279_sausage_7400_binary.fbx | Rust | 6.51 | 1.18 | 36.94 | 0.96 | 0.01 | 0.57 | 5.06 | 0.07 | 51.51 |
| blender_279_sausage_7400_binary.fbx | ufbx | 3.14 | 1.54 | 1.00 | 0.06 | 0.00 | 0.02 | 0.19 | 0.04 | 5.98 |
| chairDesk.fbx | Rust | 6.39 | 0.38 | 3.09 | 0.61 | 0.09 | 0.00 | 0.82 | 0.07 | 11.23 |
| chairDesk.fbx | ufbx | 3.68 | 0.40 | 0.35 | 0.08 | 0.00 | 0.00 | 0.00 | 0.04 | 4.55 |
| bathtub.fbx | Rust | 6.65 | 0.36 | 1.82 | 0.48 | 0.07 | 0.00 | 0.41 | 0.06 | 9.77 |
| bathtub.fbx | ufbx | 3.53 | 0.37 | 0.29 | 0.08 | 0.00 | 0.00 | 0.00 | 0.04 | 4.32 |
| bedBunk.fbx | Rust | 5.80 | 0.51 | 9.43 | 0.68 | 0.06 | 0.00 | 1.55 | 0.05 | 17.77 |
| bedBunk.fbx | ufbx | 3.75 | 0.58 | 0.34 | 0.11 | 0.00 | 0.00 | 0.00 | 0.04 | 4.81 |
| CommonTree_Snow_2.fbx | Rust | 5.86 | 0.16 | 2.34 | 3.23 | 0.07 | 0.00 | 0.74 | 0.05 | 13.16 |
| CommonTree_Snow_2.fbx | ufbx | 3.80 | 0.40 | 0.43 | 0.29 | 0.00 | 0.00 | 0.00 | 0.03 | 4.93 |

Syntax-only exact counters: Corpus: 664.48 M, Binary: 358.18 M, ASCII: 306.30 M.
