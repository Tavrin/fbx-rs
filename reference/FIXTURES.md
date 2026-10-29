# Fixture provenance

No corpus assets, scene dumps or per-input corpus manifests are committed.
`public-manifest.json` therefore has an empty fixture list. All qualification
inputs and their licence/source/hash records live in the run's scratch folder.

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
