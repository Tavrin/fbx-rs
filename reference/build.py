#!/usr/bin/env python3
"""Build pinned oracles (CPU only, about 4 GB of memory).

Requires CARGO_TARGET_DIR (oracle executables) and SCRATCH_DIR (build
receipts; outside the repository)."""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import fcntl
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def env_dir(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"{name} is not set; see reference/README.md")
    return Path(value)


TARGET = env_dir("CARGO_TARGET_DIR")
SCRATCH = env_dir("SCRATCH_DIR")


def sha(path):
    return hashlib.file_digest(path.open("rb"), "sha256").hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--historical", type=Path, required=True)
    ap.add_argument("--converted", type=Path, required=True, help="ufbx 0.23.1 source directory bundled by the Rust ufbx 0.11.5 crate")
    ap.add_argument("--profile-source", type=Path, help="optional: the downstream loader source the import profile options were read from; checked against its pinned hash")
    ap.add_argument("--crate", type=Path, required=True)
    ap.add_argument("--python", default="python3")
    ap.add_argument("--write-coverage", action="store_true", help="explicit schema regeneration after field-coverage review")
    args = ap.parse_args()
    pins = json.loads((ROOT / "reference/pins.json").read_text())
    converted = pins["engine-converted"]
    if args.profile_source and sha(args.profile_source) != converted["profile_source_sha256"]:
        raise SystemExit("import profile source checksum mismatch")
    if sha(args.crate) != converted["crate_sha256"]:
        raise SystemExit("binding crate checksum mismatch")
    # The binding archive is verified independently of its extracted source tree.
    import tarfile
    with tarfile.open(args.crate) as archive:
        for name in ("ufbx.c", "ufbx.h"):
            data = archive.extractfile("ufbx-0.11.5/ufbx/" + name).read()
            if hashlib.sha256(data).hexdigest() != converted[name]:
                raise SystemExit("binding archive source mismatch: " + name)
    TARGET.mkdir(parents=True, exist_ok=True)
    build_lock = (TARGET / ".oracle-build.lock").open("w")
    fcntl.flock(build_lock, fcntl.LOCK_EX)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    receipt = {"compiler": subprocess.check_output(["cc", "--version"], text=True), "target": platform.platform(), "pins": pins, "profiles": {}}
    for profile, source in (("historical-raw", args.historical), ("engine-converted", args.converted)):
        pin = pins[profile]
        for name in ("ufbx.c", "ufbx.h"):
            if sha(source / name) != pin[name]:
                raise SystemExit("pin mismatch: " + str(source / name))
        dest = TARGET / profile
        dest.mkdir(exist_ok=True)
        coverage = dest / "coverage.json"
        subprocess.run([args.python, str(ROOT / "reference/generate.py"), str(source / "ufbx.h"), str(dest / "generated.inc"), str(coverage)], check=True)
        expected = ROOT / "reference" / ("coverage-" + profile + ".json")
        if args.write_coverage:
            expected.write_bytes(coverage.read_bytes())
        elif not expected.exists() or expected.read_bytes() != coverage.read_bytes():
            raise SystemExit("field coverage changed: review before --write-coverage")
        config = {
            "PROFILE": profile, "SOURCE_HASH": pin["ufbx.c"], "HEADER_HASH": pin["ufbx.h"],
            "COVERAGE_HASH": sha(coverage), "DUMPER_HASH": sha(ROOT / "reference/canon.c"),
            "COMPILER_ID": receipt["compiler"].splitlines()[0],
            "TARGET_TRIPLE": subprocess.check_output(["cc", "-dumpmachine"], text=True).strip(),
            "BUILD_FLAGS": "-std=c11 -O2 -g0 -fno-fast-math -ffp-contract=off; double ufbx_real; single-threaded default loader",
            "GENERATOR_HASH": sha(ROOT / "reference/generate.py"),
        }
        (dest / "config.h").write_text("\n".join("#define " + k + " " + json.dumps(v) for k, v in config.items()) + f"\n#define CONVERTED_PROFILE {int(profile == 'engine-converted')}\n")
        command = ["cc", "-std=c11", "-O2", "-g0", "-fno-fast-math", "-ffp-contract=off", "-Wall", "-Wextra", "-Werror", "-Wno-unused-function", "-I" + str(source), "-I" + str(dest), str(ROOT / "reference/canon.c"), str(source / "ufbx.c"), "-lm", "-lcrypto", "-o", str(dest / "fbx-canon")]
        subprocess.run(command, check=True)
        if sha(ROOT / "reference/canon.c") != config["DUMPER_HASH"]:
            raise SystemExit("dumper source changed during build; discard executable")
        options = subprocess.check_output([str(dest / "fbx-canon"), "--describe-options"], text=True)
        option_rows = [json.loads(line) for line in options.splitlines()]
        options_manifest = json.dumps({"schema": "fbx-load-options/1", "profile": profile,
            "defaults": "zero values select defaults in the hash-pinned C implementation; fn=true is deny_open; all other callbacks/users null",
            "fields": option_rows[1:-1], "section_sha256": option_rows[-1]["sha256"]}, indent=2) + "\n"
        options_path = ROOT / "reference" / ("options-" + profile + ".json")
        if args.write_coverage:
            options_path.write_text(options_manifest)
        elif not options_path.exists() or options_path.read_text() != options_manifest:
            raise SystemExit("load options changed: review before --write-coverage")
        receipt["profiles"][profile] = {"command": command, "source": str(source), "executable_sha256": sha(dest / "fbx-canon"), **config}
    receipt_text = json.dumps(receipt, indent=2) + "\n"
    archive = SCRATCH / "build-receipts" / (hashlib.sha256(receipt_text.encode()).hexdigest() + ".json")
    archive.parent.mkdir(exist_ok=True)
    if not archive.exists():
        archive.write_text(receipt_text)
    (SCRATCH / "build-receipt.json").write_text(receipt_text)


if __name__ == "__main__":
    main()
