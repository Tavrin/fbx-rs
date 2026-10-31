#!/usr/bin/env python3
"""Compare the order of every public list between ufbx and fbx-rs.

The canonical streams are keyed by object identity, so a value-by-value
comparison cannot see the sequence in which ufbx exposes `nodes`, `materials`,
`anim_stacks` and the other typed lists. Both dumpers therefore end the stream
with an `order` section (see SCHEMA.md). This script dumps each input with the
oracle and with `fbx-parity dump-scene`, and requires the two `order` sections
to be identical, record for record.

    compare_order.py PATHS ORACLE PARITY [--json OUT]

PATHS is a text file with one input path per line, ORACLE the engine-converted
`fbx-canon` executable and PARITY the `fbx-parity` executable. The exit status
is 0 when every file has identical order and 1 otherwise.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

PREFIX = b'{"kind":"field","path":"/order/'


def order_records(stdout):
    """The `order` field lines of a stream, as raw bytes."""
    return [line for line in stdout.split(b"\n") if line.startswith(PREFIX)]


def dump(command, path):
    run = subprocess.run(command + [path], capture_output=True)
    return run.returncode, run.stdout, run.stderr.decode(errors="replace").strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", type=Path)
    ap.add_argument("oracle", type=Path)
    ap.add_argument("parity", type=Path)
    ap.add_argument("--json", type=Path, help="write the per-file result")
    args = ap.parse_args()
    files = [line.strip() for line in args.paths.read_text().splitlines() if line.strip()]
    results = []
    for path in files:
        code_o, out_o, err_o = dump([str(args.oracle)], path)
        code_r, out_r, err_r = dump([str(args.parity), "dump-scene"], path)
        if code_o != 0 or code_r != 0:
            results.append({"path": path, "status": "error", "oracle_exit": code_o, "rust_exit": code_r, "stderr": (err_o or err_r)[:200]})
            continue
        a, b = order_records(out_o), order_records(out_r)
        if not a or not b:
            results.append({"path": path, "status": "error", "reason": "no order section"})
        elif a == b:
            results.append({"path": path, "status": "equal", "records": len(a)})
        else:
            first = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            results.append({
                "path": path,
                "status": "differs",
                "oracle_records": len(a),
                "rust_records": len(b),
                "first_difference": [a[first].decode() if first < len(a) else None, b[first].decode() if first < len(b) else None],
            })
    counts = {}
    for r in results:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    print(json.dumps({"files": len(results), **counts}))
    for r in results:
        if r["status"] != "equal":
            print(json.dumps(r), file=sys.stderr)
    if args.json:
        args.json.write_text(json.dumps({"counts": counts, "results": results}, indent=1) + "\n")
    return 0 if counts.get("equal", 0) == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
