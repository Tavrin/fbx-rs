#!/usr/bin/env python3
"""Build the historical retained-DOM diagnostic (CPU only).

Requires FBX_HISTORICAL (the pinned ufbx 0.11.0 source directory),
CARGO_TARGET_DIR (the executable) and SCRATCH_DIR (the build receipt)."""
import hashlib
import json
import os
from pathlib import Path
import subprocess


def env_dir(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f'{name} is not set; see reference/README.md')
    return Path(value)


root = Path(__file__).resolve().parent.parent
source = env_dir('FBX_HISTORICAL')
target = env_dir('CARGO_TARGET_DIR')
scratch = env_dir('SCRATCH_DIR')
def digest(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()
pin = json.loads((root / 'reference/pins.json').read_text())['historical-raw']
for name in ('ufbx.c', 'ufbx.h'):
    if digest(source / name) != pin[name]:
        raise SystemExit('source pin mismatch: ' + name)
target.mkdir(parents=True, exist_ok=True)
scratch.mkdir(parents=True, exist_ok=True)
dumper = root / 'reference/dom.c'
before = digest(dumper)
command = ['cc', '-std=c11', '-O2', '-fno-fast-math', '-ffp-contract=off', '-Wall', '-Wextra', '-Werror', '-I' + str(source), str(dumper), str(source / 'ufbx.c'), '-lm', '-o', str(target / 'dom-oracle')]
subprocess.run(command, check=True)
if digest(dumper) != before:
    raise SystemExit('dumper changed during build')
receipt = {'profile': pin, 'dumper_sha256': before, 'executable_sha256': digest(target / 'dom-oracle'), 'command': command, 'compiler': subprocess.check_output(['cc', '--version'], text=True).splitlines()[0]}
(scratch / 'dom-build.json').write_text(json.dumps(receipt, indent=2) + '\n')
