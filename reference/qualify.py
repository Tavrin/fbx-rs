#!/usr/bin/env python3
"""Public-only repeat, corruption and profile qualification; all data in scratch.

Requires SCRATCH_DIR (inputs, dumps and receipts; outside the repository) and
CARGO_TARGET_DIR (the built oracles and fbx-parity)."""
import argparse
import ctypes
import gzip
import hashlib
import json
import os
import resource
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def env_dir(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"{name} is not set; see reference/README.md")
    return Path(value)


SCRATCH = env_dir("SCRATCH_DIR")
TARGET = env_dir("CARGO_TARGET_DIR")
PIN = "5d61cfce6272523f7a51b9d8580e156b352e1247"


def authored_regression_fixture():
    """Self-authored MIT OR Apache-2.0; no third-party scene data."""
    lines = ["; FBX 6.1.0 project file", "FBXHeaderExtension: {", "  FBXHeaderVersion: 1003", "  FBXVersion: 6100", "}", "Objects: {"]
    for name in ("A0", "A"):
        lines.extend([f'  Model: "Model::{name}", "Null" {{', "    Version: 232", "    Properties60: {", '      Property: "Lcl Translation", "Lcl Translation", "A+", 0, 0, 0', "    }", "  }"])
    lines.extend(['  Material: "Material::Distinct", "" {', "    Version: 102", '    ShadingModel: "phong"', "    MultiLayer: 0", "    Properties60: {", '      Property: "DiffuseColor", "ColorRGB", "", 0.125, 0.25, 0.5', '      Property: "DiffuseFactor", "Number", "", 1', "    }", "  }", "}", "Connections: {"])
    for name in ("A0", "A"):
        lines.append(f'  Connect: "OO", "Model::{name}", "Model::Scene"')
    lines.extend(['  Connect: "OO", "Material::Distinct", "Model::A"', "}"])
    return ("\n".join(lines) + "\n").encode()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def limits():
    resource.setrlimit(resource.RLIMIT_AS, (3584 * 1024**2,) * 2)


def run_child(command, stdout, stderr, timeout=180):
    # Become a subreaper: killing just Popen's leader leaves grandchildren as
    # zombies. After group termination, adopt and reap those descendants too.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # Linux PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")
    start = time.monotonic()
    timed_out = False
    with Path(stdout).open("wb") as out, Path(stderr).open("wb") as err:
        process = subprocess.Popen(command, stdout=out, stderr=err, start_new_session=True, preexec_fn=limits)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                pass
            code = 124
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            deadline = time.monotonic() + 5
            while True:
                try:
                    pid, _ = os.waitpid(-process.pid, os.WNOHANG)
                except ChildProcessError:
                    break
                if pid == 0:
                    if time.monotonic() > deadline:
                        raise RuntimeError("process-group descendants did not reap")
                    time.sleep(0.01)
    return {"command": list(map(str, command)), "exit_code": code, "timeout": timed_out,
            "wall_seconds": time.monotonic() - start}


def prepare(args):
    rows = [json.loads(line) for line in args.manifest.read_text().splitlines()]
    public = []
    for row in rows:
        if not row.get("redistributable"):
            continue
        path = Path(row["path"])
        if not row.get("licence") or not row.get("source_url") or not row.get("licence_evidence"):
            raise RuntimeError("public input lacks provenance: " + str(path))
        if digest(path) != row["sha256"] or path.stat().st_size != row["size"]:
            raise RuntimeError("input hash/size drift: " + str(path))
        if digest(Path(row["licence_evidence"])) != row["licence_evidence_sha256"]:
            raise RuntimeError("licence evidence drift: " + str(path))
        public.append(row)
    # Qualification requires public embedded and external texture coverage.
    # These are tiny regression assets under the same pinned
    # repository root licence; never admit the separate benchmark datasets.
    tree = json.loads(args.tree.read_text())
    if tree["sha"] != PIN or tree.get("truncated"):
        raise RuntimeError("upstream tree pin/truncation mismatch")
    names = [item["path"] for item in tree["tree"] if item["path"].startswith("data/") and item["path"].endswith(".fbx")
             and (Path(item["path"]).name == "blender_293_embedded_textures_7400_binary.fbx" or Path(item["path"]).name.startswith("maya_textured_cube_"))]
    if not names or not any("embedded_textures" in n for n in names):
        raise RuntimeError("required media fixtures absent from pinned tree")
    folder = SCRATCH / "public-additions"
    folder.mkdir(exist_ok=True)
    base = "https://raw.githubusercontent.com/ufbx/ufbx/" + PIN + "/"
    licence = folder / "LICENSE"
    if not licence.exists():
        licence.write_bytes(urllib.request.urlopen(base + "LICENSE", timeout=30).read())
    # Licence bytes must match the independently retained corpus evidence.
    old_licences = {r["licence_evidence_sha256"] for r in public if r["group"] == "ufbx-regressions"}
    if digest(licence) not in old_licences:
        raise RuntimeError("pinned upstream licence changed")
    tree_entries = {r["path"]: r for r in tree["tree"]}
    for name in sorted(names):
        path = folder / Path(name).name
        if not path.exists():
            path.write_bytes(urllib.request.urlopen(base + name, timeout=30).read())
        data = path.read_bytes()
        git_oid = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if git_oid != tree_entries[name]["sha"]:
            raise RuntimeError("download does not match pinned Git tree: " + name)
        public.append({"path": str(path), "sha256": hashlib.sha256(data).hexdigest(), "size": len(data),
                       "source_url": base + name, "licence": "MIT OR Unlicense (repository root)",
                       "licence_evidence": str(licence), "licence_evidence_sha256": digest(licence),
                       "redistributable": True, "group": "ufbx-public-media",
                       "admission": "simple exporter regression in pinned repository; root grant retained; benchmark datasets excluded"})
    authored = folder / "self_authored_order_material_6100_ascii.fbx"
    data = authored_regression_fixture()
    if authored.exists() and authored.read_bytes() != data:
        raise RuntimeError("self-authored fixture changed; retain the old capture and choose a versioned name")
    if not authored.exists():
        authored.write_bytes(data)
    evidence = ROOT / "reference/FIXTURES.md"
    public.append({"path": str(authored), "sha256": digest(authored), "size": len(data),
                   "source_url": (ROOT / "reference/qualify.py").as_uri() + "#authored_regression_fixture",
                   "source_evidence_sha256": digest(ROOT / "reference/qualify.py"),
                   "licence": "MIT OR Apache-2.0", "licence_evidence": str(evidence),
                   "licence_evidence_sha256": digest(evidence), "redistributable": True,
                   "group": "self-authored-harness", "admission": "original two-node prefix ordering and distinct RGB regression; generated in scratch"})
    out = SCRATCH / "public-corpus.jsonl"
    out.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in public))
    print(json.dumps({"manifest": str(out), "paths": len(public), "unique_sha256": len({r["sha256"] for r in public}), "media_additions": len(names), "self_authored_additions": 1}))


def mutate_field(value):
    """Change one exact leaf, preserving its shape and syntactic validity."""
    if isinstance(value, bool):
        return not value
    if isinstance(value, list):
        if not value:
            return None
        changed = mutate_field(value[0])
        if changed is not None:
            return [changed, *value[1:]]
    if isinstance(value, dict):
        if "f64" in value or "f32" in value:
            key = "f64" if "f64" in value else "f32"
            return {key: f"{int(value[key], 16) ^ 1:0{len(value[key])}x}"}
        if "i64" in value and "enum" not in value:
            return {"i64": str(int(value["i64"]) + 1)}
        if "u64" in value:
            return {"u64": str(int(value["u64"]) + 1)}
        for key in ("vec2", "vec3", "vec4", "matrix", "quat"):
            if key in value:
                changed = mutate_field(value[key])
                if changed is not None:
                    return {key: changed}
    return None


def fault_proof(dumps, folder, domain_candidates):
    from test_harness import seal, unseal
    compare = TARGET / "debug/fbx-parity"
    receipts = []
    for profile in ("historical-raw", "engine-converted"):
        for domain in ("graph", "meshes", "skins", "blend_shapes", "materials", "curves"):
            chosen = None
            preferences = {"graph": ("/euler_rotation", "/visible"),
                           "meshes": ("/vertices/0", "/vertex_indices/0"),
                           "skins": ("/weights/0", "/weight"),
                           "blend_shapes": ("/position_offsets/0", "/weight"),
                           "materials": ("/value_vec4", "/value_real_arr/0"),
                           "curves": ("/keyframes/0/time", "/keyframes/0/value")}
            candidates = sorted(domain_candidates[profile][domain], key=lambda p: p.stat().st_size)
            for dump in candidates:
                with gzip.open(dump, "rb") as stream:
                    header, fields = unseal(stream.read())
                for field in fields[domain]:
                    if field["path"].endswith(("/length", "/source_ordinal")) or "/element/" in field["path"]:
                        continue
                    if not any(part in field["path"] for part in preferences[domain]):
                        continue
                    changed = mutate_field(field["value"])
                    if changed is not None:
                        chosen = (header, fields, field, changed)
                        break
                if chosen:
                    break
            if not chosen:
                raise RuntimeError("no real semantic mutation candidate: " + profile + "/" + domain)
            header, fields, field, changed = chosen
            stem = profile + "-" + domain
            original, corrupt = folder / (stem + ".expected.jsonl"), folder / (stem + ".corrupt.jsonl")
            original.write_bytes(seal(header, fields))
            field["value"] = changed
            corrupt.write_bytes(seal(header, fields))
            valid = run_child([str(compare), "validate", str(corrupt)], folder / (stem + ".validate.out"), folder / (stem + ".validate.err"))
            result = run_child([str(compare), "compare", str(original), str(corrupt)], folder / (stem + ".out"), folder / (stem + ".err"))
            result.update({"domain": domain, "profile": profile, "path": field["path"], "mutated_dump_valid": valid["exit_code"] == 0})
            if valid["exit_code"] or result["exit_code"] != 1 or field["path"] not in (folder / (stem + ".err")).read_text():
                raise RuntimeError("fault injection was not caught at the semantic field: " + stem)
            receipts.append(result)
        # Change a real count and recompute every checksum.
        with gzip.open(dumps[profile][0], "rb") as stream:
            header, fields = unseal(stream.read())
        original, corrupt = folder / (profile + "-count.expected.jsonl"), folder / (profile + "-count.corrupt.jsonl")
        original.write_bytes(seal(header, fields))
        field = next(f for f in fields["meshes"] if f["path"] == "/meshes/length")
        field["value"]["u64"] = str(int(field["value"]["u64"]) + 1)
        corrupt.write_bytes(seal(header, fields))
        result = run_child([str(compare), "compare", str(original), str(corrupt)], folder / (profile + "-count.out"), folder / (profile + "-count.err"))
        if result["exit_code"] != 1 or "/meshes/length" not in (folder / (profile + "-count.err")).read_text():
            raise RuntimeError("count mutation not detected")
        result.update({"domain": "count", "profile": profile})
        receipts.append(result)
    # Same real input, two actual source profiles: guard must fire before any
    # ordinary domain/option difference. Explicit override reaches comparison.
    for profile, name in (("historical-raw", "mix-a"), ("engine-converted", "mix-b")):
        with gzip.open(dumps[profile][0], "rb") as stream:
            (folder / (name + ".jsonl")).write_bytes(stream.read())
    args = [str(compare), "compare", str(folder / "mix-a.jsonl"), str(folder / "mix-b.jsonl")]
    guard = run_child(args, folder / "mix.out", folder / "mix.err")
    if guard["exit_code"] != 1 or "profile mixing refused" not in (folder / "mix.err").read_text():
        raise RuntimeError("profile guard failed")
    override = run_child(args[:2] + ["--allow-profile-mixing"] + args[2:], folder / "mix-override.out", folder / "mix-override.err")
    if override["exit_code"] not in (0, 1) or "profile mixing refused" in (folder / "mix-override.err").read_text() or "invalid" in (folder / "mix-override.err").read_text():
        raise RuntimeError("explicit profile override did not reach semantic comparison")
    return {"mutations": receipts, "profile_guard": guard, "explicit_override": override}


def qualify(args):
    rows = [json.loads(line) for line in args.manifest.read_text().splitlines()]
    if any(not r.get("redistributable") for r in rows):
        raise RuntimeError("qualification accepts only the public manifest")
    folder = SCRATCH / args.run
    folder.mkdir(exist_ok=False)
    bound_paths = [ROOT / name for name in ("reference/canon.c", "reference/generate.py", "reference/build.py", "reference/qualify.py", "reference/test_harness.py", "reference/hygiene.py", "reference/pins.json", "reference/coverage-historical-raw.json", "reference/coverage-engine-converted.json", "reference/options-historical-raw.json", "reference/options-engine-converted.json", "src/lib.rs", "parity/src/main.rs", "Cargo.toml", "Cargo.lock")]
    bound_paths += [TARGET / profile / "fbx-canon" for profile in ("historical-raw", "engine-converted")]
    bound_paths += [TARGET / "debug/fbx-parity", SCRATCH / "build-receipt.json", args.manifest]
    build_receipt = json.loads((SCRATCH / "build-receipt.json").read_text())
    for profile in ("historical-raw", "engine-converted"):
        built = build_receipt["profiles"][profile]
        if built["DUMPER_HASH"] != digest(ROOT / "reference/canon.c") or built["GENERATOR_HASH"] != digest(ROOT / "reference/generate.py") or built["executable_sha256"] != digest(TARGET / profile / "fbx-canon"):
            raise RuntimeError("build receipt does not bind current source/executable: " + profile)
    binding = {str(path): digest(path) for path in bound_paths}
    (folder / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    import shutil
    for source in bound_paths:
        if source.is_relative_to(ROOT):
            destination = folder / "source" / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
    shutil.copyfile(SCRATCH / "build-receipt.json", folder / "build-receipt.json")
    shutil.copyfile(args.manifest, folder / "corpus-manifest.jsonl")
    output = folder / "receipts.jsonl"
    dumps = {p: [] for p in ("historical-raw", "engine-converted")}
    identities = {p: 0 for p in dumps}
    seen = {p: set() for p in dumps}
    errors = []
    feature_counts = {p: {d: 0 for d in ("graph", "meshes", "skins", "blend_shapes", "materials", "curves")} for p in dumps}
    domain_candidates = {p: {d: [] for d in feature_counts[p]} for p in dumps}
    for index, row in enumerate(rows):
        path = Path(row["path"])
        if digest(path) != row["sha256"]:
            raise RuntimeError("input drift: " + str(path))
        for profile in dumps:
            hashes = []
            failed = False
            prefix = f"{index:04d}-{profile}"
            first = folder / (prefix + "-0.jsonl")
            for repeat in range(3):
                dest = folder / (prefix + f"-{repeat}.jsonl")
                err = folder / (prefix + f"-{repeat}.err")
                receipt = run_child([str(TARGET / profile / "fbx-canon"), str(path)], dest, err, args.timeout)
                receipt.update({"input_sha256": row["sha256"], "path_index": index, "profile": profile, "repeat": repeat, "stdout_sha256": digest(dest), "stdout_bytes": dest.stat().st_size})
                if receipt["exit_code"] == 0:
                    with dest.open() as stream:
                        header = json.loads(stream.readline())
                    if header["input_sha256"] != row["sha256"] or int(header["input_length"]) != row["size"]:
                        failed = True
                        errors.append({"kind": "receipt-input-mismatch", **receipt})
                    valid = run_child([str(TARGET / "debug/fbx-parity"), "validate", str(dest)], folder / (prefix + f"-{repeat}.validate.out"), folder / (prefix + f"-{repeat}.validate.err"), args.timeout)
                    receipt["validation_exit_code"] = valid["exit_code"]
                    if valid["exit_code"]:
                        failed = True
                        errors.append({"kind": "invalid-dump", **receipt})
                else:
                    failed = True
                    errors.append({"kind": "oracle-failure", **receipt})
                with output.open("a") as stream:
                    stream.write(json.dumps(receipt, sort_keys=True) + "\n")
                hashes.append(receipt["stdout_sha256"])
            if digest(path) != row["sha256"]:
                failed = True
                errors.append({"kind": "input-drift-after-triplet", "profile": profile, "path_index": index})
            identical = len(set(hashes)) == 1 and not any(e["profile"] == profile and e["path_index"] == index for e in errors)
            if identical:
                identities[profile] += 1
            else:
                errors.append({"kind": "repeat-identity", "profile": profile, "path_index": index, "hashes": hashes})
            if not failed and row["sha256"] not in seen[profile]:
                # Record coverage from canonical domain counts, not filenames.
                nonempty = set()
                with first.open() as stream:
                    for line in stream:
                        v = json.loads(line)
                        for domain in feature_counts[profile]:
                            if v.get("path") == f"/{domain}/length" and int(v["value"]["u64"]) > 0:
                                feature_counts[profile][domain] += 1
                                nonempty.add(domain)
                archived = folder / (row["sha256"] + "-" + profile + ".jsonl.gz")
                with first.open("rb") as inp, archived.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                    import shutil
                    shutil.copyfileobj(inp, compressed)
                dumps[profile].append(archived)
                for domain in nonempty:
                    domain_candidates[profile][domain].append(archived)
                seen[profile].add(row["sha256"])
            if identical and not failed:
                for repeat in range(3):
                    (folder / (prefix + f"-{repeat}.jsonl")).unlink()
        if (index + 1) % 10 == 0:
            print(json.dumps({"completed_paths": index + 1, "total_paths": len(rows), "identity": identities, "errors": len(errors)}), flush=True)
    summary = {"manifest_sha256": digest(args.manifest), "public_paths": len(rows), "public_unique_sha256": len({r["sha256"] for r in rows}),
               "profiles": {p: {"identical_triplets": identities[p], "retained_unique_dumps": len(dumps[p]), "dump_runs": len(rows) * 3, "nonempty_domains_unique_inputs": feature_counts[p]} for p in dumps},
               "errors": errors, "peak_child_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss}
    (folder / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if errors:
        raise RuntimeError(f"{len(errors)} corpus errors; full receipts retained")
    faults = folder / "faults"
    faults.mkdir()
    proof = fault_proof(dumps, faults, domain_candidates)
    (folder / "fault-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    drift = [str(path) for path in bound_paths if digest(path) != binding[str(path)]]
    if drift:
        (folder / "binding-drift.json").write_text(json.dumps(drift, indent=2) + "\n")
        raise RuntimeError("qualification source/executable binding changed")
    print(json.dumps(summary), flush=True)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--manifest", type=Path, required=True)
    prep.add_argument("--tree", type=Path, required=True)
    run = sub.add_parser("run")
    run.add_argument("--manifest", type=Path, default=SCRATCH / "public-corpus.jsonl")
    run.add_argument("--run", required=True, help="new scratch subdirectory; never overwrite receipts")
    run.add_argument("--timeout", type=int, default=180)
    args = ap.parse_args()
    SCRATCH.mkdir(parents=True, exist_ok=True)
    if args.command == "prepare":
        prepare(args)
    else:
        qualify(args)


if __name__ == "__main__":
    main()
