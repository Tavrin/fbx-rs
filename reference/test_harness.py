#!/usr/bin/env python3
"""Independent stream/admission tests. No corpus data is embedded here."""
import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def env_dir(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"{name} is not set; see reference/README.md")
    return Path(value)


TARGET = env_dir("CARGO_TARGET_DIR")
SCRATCH = env_dir("SCRATCH_DIR")
SECTIONS = ["options", "globals", "dom", "elements", "connections", "graph", "meshes", "skins", "blend_shapes", "materials", "cameras_lights", "curves", "advanced", "triangles", "curve_samples", "evaluated"]


def encode(v):
    return (json.dumps(v, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def seal(receipt, fields):
    """Recompute integrity independently, so semantic mutations test comparison."""
    result = bytearray(encode(receipt))
    total = 0
    for section in SECTIONS:
        result += encode({"kind": "begin", "name": section})
        data = b"".join(encode(f) for f in fields[section])
        result += data
        result += encode({"kind": "section", "name": section, "count": str(len(fields[section])), "sha256": hashlib.sha256(data).hexdigest()})
        total += len(fields[section])
    result += encode({"kind": "footer", "records": str(total), "sha256": hashlib.sha256(result).hexdigest(), "exit_status": "0"})
    return bytes(result)


def unseal(data):
    rows = [json.loads(line) for line in data.splitlines()]
    fields = {s: [] for s in SECTIONS}
    for row in rows:
        if row["kind"] == "field":
            fields[row["path"].split("/")[1]].append(row)
    return rows[0], fields


def example(profile="historical-raw"):
    pins = json.loads((ROOT / "reference/pins.json").read_text())[profile]
    coverage = hashlib.sha256((ROOT / ("reference/coverage-" + profile + ".json")).read_bytes()).hexdigest()
    receipt = {"kind": "receipt", "schema": "fbx-scene-canon/1", "profile": profile,
               "source_sha256": pins["ufbx.c"], "header_sha256": pins["ufbx.h"],
               "coverage_sha256": coverage, "dumper_sha256": "1" * 64,
               "input_sha256": "2" * 64, "input_length": "0", "exit_status": "0",
               "real": "f64", "external_open": "denied"}
    fields = {s: [{"kind": "field", "path": "/" + s + "/length", "value": {"u64": "1"}}] for s in SECTIONS}
    fields["options"] = json.loads((ROOT / ("reference/options-" + profile + ".json")).read_text())["fields"]
    fields["elements"].append({"kind": "field", "path": "/elements/0", "value": {"ref": "id:0:ordinal:0"}})
    for s in ["graph", "meshes", "skins", "blend_shapes", "materials", "curves"]:
        fields[s].append({"kind": "field", "path": "/" + s + "/id:0/value", "value": {"f64": "3ff0000000000000"}})
    return receipt, fields


class StreamTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="stream-test-", dir=SCRATCH)
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.exe = TARGET / "debug/fbx-parity"

    def command(self, *args):
        return subprocess.run([str(self.exe), *map(str, args)], capture_output=True, text=True, timeout=30)

    def write(self, name, receipt=None, fields=None, data=None):
        path = self.directory / name
        if data is None:
            r, f = example()
            data = seal(receipt or r, fields or f)
        path.write_bytes(data)
        return path

    def test_valid_and_equal(self):
        a = self.write("a")
        self.assertEqual(self.command("validate", a).returncode, 0)
        self.assertEqual(self.command("compare", a, a).returncode, 0)

    def test_every_domain_mutation_survives_integrity_and_is_detected(self):
        a = self.write("a")
        for domain in ["graph", "meshes", "skins", "blend_shapes", "materials", "curves"]:
            r, f = example()
            f[domain][1]["value"]["f64"] = "3ff0000000000001"
            b = self.write("b", r, f)
            self.assertEqual(self.command("validate", b).returncode, 0)
            result = self.command("compare", a, b)
            self.assertEqual(result.returncode, 1)
            self.assertIn("/" + domain + "/id:0/value", result.stderr)

    def test_counts_and_missing_fields(self):
        a = self.write("a")
        for domain in ["graph", "meshes", "skins", "blend_shapes", "materials", "curves"]:
            r, f = example()
            f[domain][0]["value"]["u64"] = "2"
            b = self.write("b", r, f)
            result = self.command("compare", a, b)
            self.assertEqual(result.returncode, 1)
            self.assertIn("/" + domain + "/length", result.stderr)
        r, f = example()
        f["curves"].pop()
        b = self.write("b", r, f)
        self.assertIn("record count differs", self.command("compare", a, b).stderr)

    def test_profile_guard_and_explicit_override(self):
        a = self.write("a")
        r, f = example("engine-converted")
        b = self.write("b", r, f)
        result = self.command("compare", a, b)
        self.assertEqual(result.returncode, 1)
        self.assertIn("profile mixing refused", result.stderr)
        self.assertEqual(self.command("compare", "--allow-profile-mixing", a, b).returncode, 0)

    def test_changed_options_are_exact(self):
        a = self.write("a")
        r, f = example()
        f["options"][0]["value"] = {"u64": "2"}
        b = self.write("b", r, f)
        self.assertIn("pinned load options mismatch", self.command("compare", a, b).stderr)

    def test_truncation_and_integrity_even_after_early_difference(self):
        a = self.write("a")
        data = a.read_bytes()
        for bad in [data[:-1], data[:data.rfind(b'{"kind":"footer"')], data + b"{}\n", data.replace(b'"u64":"1"', b'"u64":"2"', 1)]:
            b = self.write("b", data=bad)
            result = self.command("compare", a, b)
            self.assertEqual(result.returncode, 1)
            self.assertIn("dump invalid", result.stderr)

    def test_section_order_pin_and_duplicate_keys(self):
        r, f = example()
        r["source_sha256"] = "0" * 64
        bad = self.write("b", r, f)
        self.assertIn("pin mismatch", self.command("validate", bad).stderr)
        data = seal(*example())
        bad = self.write("b", data=data.replace(b'"kind":"receipt"', b'"kind":"receipt","kind":"receipt"', 1))
        self.assertIn("duplicate key", self.command("validate", bad).stderr)
        bad = self.write("b", data=data.replace(b'"name":"options"', b'"name":"globals"', 1))
        self.assertIn("section order", self.command("validate", bad).stderr)

    def test_nonfinite_and_forged_tolerance(self):
        r, f = example()
        for path, values in [("/curve_samples/id:0/0/value", ["7ff0000000000000"]),
                             ("/curves/id:0/value", ["3ff0000000000000"])]:
            fields = copy.deepcopy(f)
            section = path.split("/")[1]
            fields[section].append({"kind": "field", "path": path, "value": {"eval_f64": values}})
            bad = self.write("b", r, fields)
            self.assertEqual(self.command("validate", bad).returncode, 1)

    def test_duplicate_identities_and_dangling_references(self):
        r, f = example()
        f["elements"].append({"kind": "field", "path": "/elements/1", "value": {"ref": "id:0:ordinal:0"}})
        bad = self.write("b", r, f)
        self.assertIn("duplicate source identity", self.command("validate", bad).stderr)
        r, f = example()
        f["graph"].append({"kind": "field", "path": "/graph/id:0/parent", "value": {"ref": "missing"}})
        bad = self.write("b", r, f)
        self.assertIn("dangling source reference", self.command("validate", bad).stderr)

    def test_hygiene_rejects_unlisted_and_disguised_artifacts(self):
        sys.path.insert(0, str(ROOT / "reference"))
        from hygiene import check
        root = self.directory
        (root / "reference").mkdir()
        (root / "reference/FIXTURES.md").write_text("No fixtures.\n")
        manifest = {"source_paths": ["reference/FIXTURES.md", "renamed.rs"], "fixtures": []}
        (root / "unlisted.txt").write_text("unlisted\n")
        self.assertTrue(any("unlisted.txt" in e for e in check(root, manifest, set())))
        (root / "unlisted.txt").unlink()
        (root / "renamed.rs").write_bytes(seal(*example()))
        self.assertTrue(any("canonical scene data" in e for e in check(root, manifest, set())))
        (root / "renamed.rs").write_bytes(b"known input")
        known = {hashlib.sha256(b"known input").hexdigest()}
        self.assertTrue(any("known corpus input" in e for e in check(root, manifest, known)))

    def test_timeout_kills_and_reaps_descendants(self):
        sys.path.insert(0, str(ROOT / "reference"))
        from qualify import run_child
        pidfile = self.directory / "pid"
        program = "import os,subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(30)"
        result = run_child([sys.executable, "-c", program, str(pidfile)], self.directory / "out", self.directory / "err", timeout=0.3)
        self.assertTrue(result["timeout"])
        self.assertNotEqual(result["exit_code"], 0)
        pid = int(pidfile.read_text())
        self.assertFalse(Path("/proc", str(pid)).exists(), "descendant must be killed AND reaped")

    def authored_dump(self, profile):
        sys.path.insert(0, str(ROOT / "reference"))
        from qualify import authored_regression_fixture, run_child
        fixture = self.directory / "authored.fbx"
        fixture.write_bytes(authored_regression_fixture())
        output = self.directory / (profile + ".jsonl")
        error = self.directory / (profile + ".err")
        result = run_child([str(TARGET / profile / "fbx-canon"), str(fixture)], output, error)
        self.assertEqual(result["exit_code"], 0, error.read_text())
        self.assertEqual(self.command("validate", output).returncode, 0)
        return [json.loads(line) for line in output.read_text().splitlines()]

    def test_legacy_prefix_names_sort_by_source_bytes(self):
        for profile in ("historical-raw", "engine-converted"):
            rows = self.authored_dump(profile)
            keys = [r["value"]["ref"] for r in rows if r.get("path", "").startswith("/elements/") and isinstance(r.get("value"), dict) and "ref" in r["value"]]
            names = [bytes.fromhex(k.split(":")[2]) for k in keys if k.startswith("legacy:" + b"Model".hex() + ":")]
            self.assertEqual(names, [b"Model::A", b"Model::A0"])

    def test_material_maps_preserve_nonfirst_colour_components(self):
        import struct
        for profile in ("historical-raw", "engine-converted"):
            rows = self.authored_dump(profile)
            maps = [r for r in rows if r.get("path", "").startswith("/materials/") and r["path"].endswith("/fbx/diffuse_color/value_vec4")]
            self.assertEqual(len(maps), 1)
            components = maps[0]["value"]["vec4"]
            expected = [struct.pack(">d", x).hex() for x in (0.125, 0.25, 0.5)]
            self.assertEqual([v["f64"] for v in components[:3]], expected)
            self.assertEqual(len(components), 4)


if __name__ == "__main__":
    SCRATCH.mkdir(parents=True, exist_ok=True)
    unittest.main()
