# Fuzz seed preparation

Before the separate pre-release fuzz campaign, add the qualified corpus seeds
as described in `docs/VERIFICATION.md`, then add the authored legacy seeds to
each persistent target corpus:

```sh
python3 fuzz/make_legacy_seeds.py fuzz/corpus/binary
python3 fuzz/make_legacy_seeds.py fuzz/corpus/ascii
python3 fuzz/make_legacy_seeds.py fuzz/corpus/scene_dom
python3 fuzz/make_legacy_seeds.py fuzz/corpus/load
```

The generator reproduces little- and big-endian binary files containing
compressed geometry and mixed scalar key records. It also adds the authored
Properties60/take, named skin/pose/nested shape, material texture-layer and
root-model/UV ASCII fixtures from `tests/fixtures`, plus a version-3000 binary
container declaring FBX 5800.
It does not run a fuzzer. Generated corpora remain ignored build inputs.
