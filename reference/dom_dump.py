#!/usr/bin/env python3
"""Bind the historical retained-DOM diagnostic to input and build hashes.

This is a preliminary stream, not the corpus equivalence comparator. Source FBX
scalar type codes discarded by ufbx cannot be reconstructed from its DOM.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

def env_dir(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f'{name} is not set; see reference/README.md')
    return Path(value)


SCRATCH = env_dir('SCRATCH_DIR')
TARGET = env_dir('CARGO_TARGET_DIR')
ROOT = Path(__file__).resolve().parent.parent
MAX_LINE = 64 * 1024 * 1024

def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def encoded(obj):
    return (json.dumps(obj, ensure_ascii=True, separators=(',', ':')) + '\n').encode('ascii')

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('input', type=Path)
    ap.add_argument('output', type=Path)
    args = ap.parse_args()
    receipt = json.loads((SCRATCH / 'dom-build.json').read_text())
    exe = TARGET / 'dom-oracle'
    dumper = ROOT / 'reference/dom.c'
    if sha(exe) != receipt['executable_sha256'] or sha(dumper) != receipt['dumper_sha256']:
        raise SystemExit('oracle build binding mismatch')
    runner_hash = sha(Path(__file__))
    build_hash = sha(SCRATCH / 'dom-build.json')
    before = sha(args.input)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryFile(dir=SCRATCH) as raw:
        result = subprocess.run([str(exe), str(args.input)], stdout=raw, stderr=subprocess.PIPE, timeout=180)
        if result.returncode:
            raise SystemExit('oracle rejected input: ' + result.stderr.decode('utf8', errors='replace'))
        if sha(args.input) != before or sha(exe) != receipt['executable_sha256'] or sha(Path(__file__)) != runner_hash or sha(SCRATCH / 'dom-build.json') != build_hash:
            raise SystemExit('input or executable changed during oracle run')
        raw.seek(0)
        if raw.readline(32) != b'fbx-ufbx-dom/2\n':
            raise SystemExit('invalid oracle header')
        args.output.parent.mkdir(parents=True, exist_ok=True)
        if args.output.exists(): raise SystemExit('output already exists')
        temporary = args.output.with_suffix(args.output.suffix + '.tmp')
        digest = hashlib.sha256(); count = 0
        with temporary.open('xb') as out:
            header = encoded({'kind':'header','schema':'fbx-ufbx-dom/2','input_sha256':before,'input_length':args.input.stat().st_size,'source_sha256':receipt['profile']['ufbx.c'],'header_sha256':receipt['profile']['ufbx.h'],'dumper_sha256':receipt['dumper_sha256'],'executable_sha256':receipt['executable_sha256'],'runner_sha256':runner_hash,'build_receipt_sha256':build_hash,'retain_dom':True,'external_files':False,'open_main_file_with_default':True,'file_format':'FBX','temp_allocator_bytes':805306368,'result_allocator_bytes':805306368})
            out.write(header); digest.update(header)
            while True:
                line = raw.readline(MAX_LINE + 1)
                if not line: break
                if len(line) > MAX_LINE or not line.endswith(b'\n'):
                    raise SystemExit('oracle line exceeds bound or is truncated')
                parts = line[:-1].decode('ascii').split(' ')
                if parts[0] == 'N' and len(parts) == 5:
                    row = {'kind':'node','depth':int(parts[1]),'name_hex':parts[2],'values':int(parts[3]),'children':int(parts[4])}
                elif parts[0] == 'V' and len(parts) == 6:
                    row = {'kind':'value','type':int(parts[1]),'integer':parts[2],'float64_hex':parts[3],'string_hex':parts[4],'blob_hex':parts[5]}
                else: raise SystemExit('invalid oracle record')
                data = encoded(row); out.write(data); digest.update(data); count += 1
            out.write(encoded({'kind':'footer','records':count,'sha256':digest.hexdigest(),'oracle_exit':result.returncode}))
        os.replace(temporary, args.output)

if __name__ == '__main__': main()
