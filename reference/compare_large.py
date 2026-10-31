#!/usr/bin/env python3
"""Compare whole canonical streams of large files, section by section.

A production-size file produces tens of GiB of canonical stream, so neither
dump is held in memory or compared field by field in one pass. Each dump is
piped into `fbx-parity sections --save DIR`, which validates the stream in a
single pass (framing, checksums, identities) and writes every section to
`DIR/NAME.jsonl`. Two sections are equal exactly when their count and SHA-256
agree, so `evaluated` and `order` are compared record for record, in order,
bit for bit. Equal sections are deleted at once; a differing section is kept
and either classified or its first differing records are printed.

Two kinds of difference are classified rather than failed, and are named in
the output: `dom` records that are numeric array blocks of identical path and
length (ufbx normalises normals and clamps indices inside its retained DOM,
where fbx-rs keeps the file's content), and `globals` records under
`metadata/warnings` and `metadata/has_warning` (fbx-rs does not reproduce
ufbx's warning list) or `settings/axes` (ufbx leaves the file's axes there
after converting, fbx-rs records the axes of the loaded scene). Anything
else is a failure.

    compare_large.py ORACLE PARITY WORKDIR FBX... [--timeout SECONDS]
                     [--min-free-gb N] [--sequential] [--json OUT]

`--sequential` runs the oracle and then the Rust dump instead of both at once,
which halves the peak memory on a file of several hundred MiB.

ORACLE is the engine-converted `fbx-canon` executable, PARITY the
`fbx-parity` executable. The exit status is 0 when every file is equal in
every section or differs only in those classes, and 1 otherwise (including a
dump that fails or times out).
"""
import argparse
import json
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path


def parse_sections(text):
    sections, records = {}, None
    for line in text.splitlines():
        name, *rest = line.split()
        if name == "records":
            records = int(rest[0])
        else:
            sections[name] = (int(rest[0]), rest[1])
    return sections, records


def first_differences(a, b, limit=3):
    """The first `limit` differing (line number, oracle line, rust line)."""
    found = []
    with open(a, "rb") as fa, open(b, "rb") as fb:
        n = 0
        while len(found) < limit:
            la, lb = fa.readline(), fb.readline()
            if not la and not lb:
                break
            n += 1
            if la != lb:
                found.append((n, la[:400].decode(errors="replace").rstrip(), lb[:400].decode(errors="replace").rstrip()))
    return found


def read_names(path):
    """Map each DOM node path prefix to its name, from `/name` records."""
    names = {}
    with open(path, "rb") as f:
        for line in f:
            if b'/name","value"' in line:
                rec = json.loads(line)
                names[rec["path"][: -len("/name")]] = bytes.fromhex(rec["value"]["bytes_hex"]).decode("latin1")
    return names


def classify_dom(oracle, rust):
    """Classify the differing `dom` records.

    ufbx modifies some retained DOM arrays in place while loading (normals are
    normalised, out-of-range indices are clamped), so the oracle's DOM differs
    from the file's content, which fbx-rs keeps. That is the only difference
    accepted here, and only for a record that is a numeric array element block
    with identical path and length. Returns (counts by node name, unexplained).
    """
    names = read_names(rust)
    counts, unexplained = {}, []
    with open(oracle, "rb") as fa, open(rust, "rb") as fb:
        for la, lb in zip(fa, fb):
            if la == lb:
                continue
            a, b = json.loads(la), json.loads(lb)
            owner = a["path"].rsplit("/values/", 1)[0]
            va, vb = a["value"], b["value"]
            blocks = (
                a["path"] == b["path"]
                and "/array/" in a["path"]
                and isinstance(va, list)
                and isinstance(vb, list)
                and len(va) == len(vb)
            )
            if blocks:
                key = names.get(owner, "?")
                counts[key] = counts.get(key, 0) + 1
            elif len(unexplained) < 5:
                unexplained.append((la[:300].decode(errors="replace"), lb[:300].decode(errors="replace")))
            else:
                unexplained.append(None)
    return counts, [u for u in unexplained if u]


def warning_only(oracle, rust):
    """True when the two `globals` sections differ only in ufbx's warning list
    (`metadata/warnings`), its per-type flags (`metadata/has_warning`) and
    `settings/axes`."""
    def records(path):
        out = {}
        with open(path, "rb") as f:
            for line in f:
                rec = json.loads(line)
                out[rec["path"]] = line
        return out

    a, b = records(oracle), records(rust)
    changed = {p for p in set(a) | set(b) if a.get(p) != b.get(p)}
    known_paths = (
        "/globals/metadata/warnings",
        "/globals/metadata/has_warning",
        # ufbx leaves the file's axes in settings.axes after converting; fbx-rs
        # records the axes of the loaded scene (the target axes).
        "/globals/settings/axes",
    )
    return all(p.startswith(known_paths) for p in changed), len(changed)


def run_pair(oracle, parity, fbx, workdir, timeout, min_free, sequential=False):
    """Run both dumps, concurrently or one after the other (peak memory is then
    the larger of the two); return (result dict, killed reason, work dirs)."""
    dirs = {"oracle": workdir / "oracle", "rust": workdir / "rust"}
    for d in dirs.values():
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True)
    commands = {
        "oracle": [str(oracle), fbx],
        "rust": [str(parity), "dump-scene", "--max-output", "0", fbx],
    }
    procs, sinks, started = {}, {}, time.monotonic()
    finished = threading.Event()
    killed = []

    def watch():
        while not finished.is_set():
            if time.monotonic() - started > timeout:
                killed.append(f"timeout after {timeout} s")
            elif shutil.disk_usage(workdir).free < min_free << 30:
                killed.append(f"free space below {min_free} GiB")
            if killed:
                for p in list(procs.values()) + list(sinks.values()):
                    if p.poll() is None:
                        p.kill()
                return
            time.sleep(2)

    threading.Thread(target=watch, daemon=True).start()
    out = {}
    for who, cmd in commands.items():
        dump = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        sink = subprocess.Popen(
            [str(parity), "sections", "--save", str(dirs[who])],
            stdin=dump.stdout, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        dump.stdout.close()
        procs[who], sinks[who] = dump, sink
        if sequential:
            collect(who, procs, sinks, out, started)
    for who in commands:
        if who not in out:
            collect(who, procs, sinks, out, started)
    finished.set()
    return out, (killed[0] if killed else None), dirs


def collect(who, procs, sinks, out, started):
    """Wait for one dump and its section sink and record the outcome."""
    text, sink_err = sinks[who].communicate()
    err = procs[who].stderr.read().decode(errors="replace").strip()
    procs[who].wait()
    out[who] = {
        "dump_exit": procs[who].returncode,
        "sink_exit": sinks[who].returncode,
        "stderr": (err or sink_err.decode(errors="replace").strip())[:300],
        "text": text.decode(),
        "seconds": round(time.monotonic() - started, 1),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("oracle", type=Path)
    ap.add_argument("parity", type=Path)
    ap.add_argument("workdir", type=Path)
    ap.add_argument("files", nargs="+")
    ap.add_argument("--timeout", type=int, default=14400, help="seconds per file for both dumps")
    ap.add_argument("--min-free-gb", type=int, default=15)
    ap.add_argument("--sequential", action="store_true", help="run the oracle, then the Rust dump")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    args.workdir.mkdir(parents=True, exist_ok=True)
    results, failed = [], False
    for fbx in args.files:
        out, killed, dirs = run_pair(args.oracle, args.parity, fbx, args.workdir, args.timeout, args.min_free_gb, args.sequential)
        entry = {"path": fbx, "seconds": {w: out[w]["seconds"] for w in out}}
        print(f"== {fbx}", flush=True)
        broken = killed or any(o["dump_exit"] != 0 or o["sink_exit"] != 0 for o in out.values())
        if broken:
            failed = True
            entry["status"] = "error"
            entry["error"] = killed or {w: {k: o[k] for k in ("dump_exit", "sink_exit", "stderr")} for w, o in out.items()}
            print(f"  ERROR {entry['error']}", flush=True)
        else:
            (so, ro), (sr, rr) = parse_sections(out["oracle"]["text"]), parse_sections(out["rust"]["text"])
            entry["sections"], entry["records"] = {}, {"oracle": ro, "rust": rr}
            for name in so:
                same = so[name] == sr.get(name)
                entry["sections"][name] = {"oracle": so[name], "rust": sr.get(name), "equal": same}
                line = f"  {name:15} {'EQUAL' if same else 'DIFF '} records={so[name][0]}"
                if same:
                    for who in dirs:
                        (dirs[who] / f"{name}.jsonl").unlink(missing_ok=True)
                else:
                    line += f" rust_records={sr.get(name, (None,))[0]}"
                    po, pr = dirs["oracle"] / f"{name}.jsonl", dirs["rust"] / f"{name}.jsonl"
                    known = None
                    if name == "dom" and so[name][0] == sr.get(name, (None,))[0]:
                        counts, odd = classify_dom(po, pr)
                        if not odd:
                            known = f"explained: ufbx mutates retained DOM arrays in place; {counts}"
                    elif name == "globals":
                        only, changed = warning_only(po, pr)
                        if only:
                            known = f"known: ufbx warning list and file axes not reproduced ({changed} paths)"
                    if known:
                        entry["sections"][name]["known_difference"] = known
                        line += f"\n    {known}"
                    else:
                        failed = True
                        entry["sections"][name]["first_differences"] = first_differences(po, pr)
                print(line, flush=True)
                for n, a, b in entry["sections"][name].get("first_differences", []):
                    print(f"    line {n}\n      oracle {a}\n      rust   {b}", flush=True)
            entry["status"] = (
                "equal"
                if all(s["equal"] for s in entry["sections"].values())
                else "known-differences"
                if all(s["equal"] or "known_difference" in s for s in entry["sections"].values())
                else "differs"
            )
            print(f"  seconds oracle={out['oracle']['seconds']} rust={out['rust']['seconds']} records oracle={ro} rust={rr}", flush=True)
        results.append(entry)
        if entry["status"] in ("equal", "known-differences"):
            for d in dirs.values():
                shutil.rmtree(d, ignore_errors=True)
    if args.json:
        args.json.write_text(json.dumps(results, indent=1) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
