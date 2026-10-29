#!/usr/bin/env python3
"""Fail closed on unlisted repository files and corpus-derived data.

The working tree (including ignored/untracked files), index and reachable Git
blobs are inspected. Only fixtures explicitly admitted by public-manifest.json
may contain corpus bytes; the default manifest admits none.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def corpus_signature(data):
    if data.startswith(b"Kaydara FBX Binary") or data.lstrip().startswith(b"; FBX"):
        return "FBX asset"
    if data.startswith((b"\x1f\x8b", b"PK\x03\x04", b"\x7fELF")):
        return "binary/archive artifact"
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return "binary data"
    for line in text.splitlines():
        if not line.startswith("{"):
            continue
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if not isinstance(row, dict):
            continue
        if row.get("schema") == "fbx-scene-canon/1" or row.get("kind") in {"receipt", "footer", "field"}:
            return "canonical scene data"
        if "probe_schema" in row or ("path" in row and "sha256" in row and "redistributable" in row):
            return "corpus manifest/probe record"
    return None


def check(root, manifest, corpus_hashes):
    allowed = set(manifest["source_paths"])
    fixtures = {f["path"]: f for f in manifest["fixtures"]}
    failures = []

    def inspect(name, data, historical=False):
        digest = hashlib.sha256(data).hexdigest()
        fixture = fixtures.get(name)
        if fixture:
            if not fixture.get("redistributable") or not fixture.get("licence") or not fixture.get("source_url") or fixture.get("sha256") != digest:
                failures.append(name + ": invalid public fixture admission")
            if name not in (root / "reference/FIXTURES.md").read_text():
                failures.append(name + ": missing FIXTURES.md source/licence line")
            return
        why = corpus_signature(data)
        if digest in corpus_hashes:
            why = "known corpus input (including renamed files)"
        if why:
            failures.append(name + ": " + why)
        if not historical and name not in allowed:
            failures.append(name + ": absent from public repository manifest")

    for file in sorted(root.rglob("*")):
        rel = file.relative_to(root)
        if rel.parts[0] == ".git":
            continue
        if file.is_symlink():
            failures.append(str(rel) + ": repository symlink is not admitted")
        elif file.is_file():
            inspect(str(rel), file.read_bytes())
    if (root / ".git").exists():
        # Include index blobs even if the working tree has subsequently changed.
        index = subprocess.check_output(["git", "ls-files", "--stage", "-z"], cwd=root).split(b"\0")
        blobs = {}
        for row in index:
            if row:
                mode_hash_stage, name = row.split(b"\t", 1)
                oid = mode_hash_stage.split()[1].decode()
                blobs[oid] = name.decode()
        history = subprocess.check_output(["git", "rev-list", "--objects", "--all"], cwd=root, text=True)
        for row in history.splitlines():
            oid, _, name = row.partition(" ")
            blobs.setdefault(oid, name)
        for oid, name in blobs.items():
            kind = subprocess.check_output(["git", "cat-file", "-t", oid], cwd=root, text=True).strip()
            if kind == "blob":
                data = subprocess.check_output(["git", "cat-file", "blob", oid], cwd=root)
                inspect(name, data, historical=True)
    return failures


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=Path, default=ROOT)
    ap.add_argument("--corpus-manifest", type=Path)
    args = ap.parse_args()
    manifest = json.loads((args.root / "reference/public-manifest.json").read_text())
    known = set()
    if args.corpus_manifest:
        known = {json.loads(line)["sha256"] for line in args.corpus_manifest.read_text().splitlines()}
    failures = check(args.root, manifest, known)
    for failure in failures:
        print(failure)
    if failures:
        raise SystemExit(1)
    print("hygiene: working tree, index and reachable history contain only admitted sources/fixtures")


if __name__ == "__main__":
    main()
