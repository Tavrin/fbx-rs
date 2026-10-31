#!/usr/bin/env python3
"""Compare baked animation between ufbx and fbx-rs, including order.

    compare_bake.py PATHS BAKE_ORACLE BAKE_DUMP [--json OUT]

PATHS is a text file with one input path per line. BAKE_ORACLE is
`reference/bake_probe.c` built against the pinned ufbx and BAKE_DUMP is
`examples/bake_dump.rs` built from this crate. Every input is baked with four
option sets (reduced, reduced and trimmed, reduced with rotation-aware
reduction and trimmed, raw and trimmed). Both programs print stacks (`S`),
baked nodes (`N`) and baked properties (`P`) in the order of the bake result.

The stream passes when the sequence of `S`, `N` and `P` records is identical
line for line (so node and property order is checked), and every key record
(`K`, `Q`) of every node and property is identical bit for bit. Key records
are grouped under their owner, because the two programs print the channels of
one node in a different order.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

OPTIONS = ["reduce", "reduce-trim", "reduce-trim-rotation", "raw-trim"]


def parse(stream):
    headers, keys = [], {}
    owner = {"N": None, "P": None}
    for line in stream.decode().splitlines():
        fields = line.split()
        kind = fields[0]
        if kind in ("S", "N", "P"):
            headers.append(line)
            if kind != "S":
                owner[kind] = len(headers) - 1
        elif kind == "K":
            # K stack id channel index values...
            keys[(owner["N"], "K", fields[3], fields[4])] = fields[5:]
        elif kind == "Q":
            keys[(owner["P"], "Q", fields[3])] = fields[3:]
    return headers, keys


def run(command, path, option):
    return subprocess.run(command + [path, option], capture_output=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", type=Path)
    ap.add_argument("oracle", type=Path)
    ap.add_argument("dump", type=Path)
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    files = [line.strip() for line in args.paths.read_text().splitlines() if line.strip()]
    results = []
    for path in files:
        for option in OPTIONS:
            a, b = run([str(args.oracle)], path, option), run([str(args.dump)], path, option)
            row = {"path": path, "options": option, "oracle_exit": a.returncode, "rust_exit": b.returncode}
            if a.returncode == 0 and b.returncode == 0:
                (ha, ka), (hb, kb) = parse(a.stdout), parse(b.stdout)
                row["order_equal"] = ha == hb
                row["keys_equal"] = ka == kb
                row["exact"] = ha == hb and ka == kb
                row["records"] = len(ha) + len(ka)
                if not row["exact"]:
                    row["headers_only_oracle"] = [h for h in ha if h not in set(hb)][:5]
                    row["headers_only_rust"] = [h for h in hb if h not in set(ha)][:5]
            else:
                row["exact"] = False
            results.append(row)
    exact = sum(r["exact"] for r in results)
    summary = {"comparisons": len(results), "exact": exact, "inputs": len(files)}
    print(json.dumps(summary))
    for r in results:
        if not r["exact"]:
            print(json.dumps(r), file=sys.stderr)
    if args.json:
        args.json.write_text(json.dumps({"summary": summary, "results": results}, indent=1) + "\n")
    return 0 if exact == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
