# Fixture provenance

External corpus assets, scene dumps and per-input corpus manifests remain
outside the repository. `public-manifest.json` admits the self-authored legacy
regressions below with exact hashes and redistribution decisions. External
qualification inputs and their licence/source/hash records live in scratch.

The comparator's generated test records are self-authored schema examples, not
recovered or transformed scene data. They carry the package's MIT OR Apache-2.0
licence. `test_harness.py` generates any test files in a temporary scratch folder.

`self_authored_order_material_6100_ascii.fbx` is generated only in scratch by
`reference/qualify.py:authored_regression_fixture()`. Its original two-node
prefix-name and distinct RGB material scene is licensed **MIT OR Apache-2.0**;
source: that generator function in this repository. It contains no third-party
scene data. The generated public corpus manifest retains its bytes/source hashes
and this licence evidence; the native harness tests exercise both oracle pins.

Before adding a public fixture, add its path, SHA-256, source URL, redistribution
decision and licence to `public-manifest.json`, and a corresponding licence and
source line here. Private data must never be admitted by that manifest.

## Authored legacy regression fixtures

Each fixture below is original scene data created for this implementation and
licensed **MIT OR Apache-2.0**. None is recovered or transformed corpus data.
The binary files are reproducible with `fuzz/make_legacy_seeds.py`; ASCII source
bytes are kept directly in the listed fixture. Their SHA-256 hashes and source
URLs are in `public-manifest.json`.

| Path | Source and coverage |
| --- | --- |
| `tests/fixtures/legacy-take.fbx` | Self-authored ASCII; named hierarchy, Properties60 and Takes |
| `tests/fixtures/legacy-root.fbx` | Self-authored ASCII; 5.8 root model, normals and UVs |
| `tests/fixtures/legacy-deform.fbx` | Self-authored ASCII; named skin/pose and nested shape |
| `tests/fixtures/legacy-texture.fbx` | Self-authored ASCII; legacy texture-slot material patch |
| `tests/fixtures/legacy-le.fbx` | `fuzz/make_legacy_seeds.py`; compressed arrays and mixed keys, little-endian |
| `tests/fixtures/legacy-be.fbx` | `fuzz/make_legacy_seeds.py`; compressed arrays and mixed keys, big-endian |
| `tests/fixtures/legacy-v5-binary.fbx` | `fuzz/make_legacy_seeds.py`; physical 3000/logical 5800 container |
