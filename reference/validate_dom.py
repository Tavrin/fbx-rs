#!/usr/bin/env python3
"""Validate the retained-ufbx DOM stream and its optional source bindings.

Validation establishes stream integrity and structure, not source-bit parity.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent
MAX_LINE = 64 * 1024 * 1024
MAX_DEPTH = 4096
HEX = re.compile(r'^(?:[0-9a-f]{2})*$')


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def pairs(items):
    out = {}
    for key, value in items:
        if key in out:
            raise ValueError('duplicate key')
        out[key] = value
    return out


def record(line):
    if not line.endswith(b'\n') or len(line) > MAX_LINE:
        raise ValueError('truncated or oversized record')
    value = json.loads(line, object_pairs_hook=pairs,
                       parse_float=lambda _: (_ for _ in ()).throw(ValueError('JSON float')),
                       parse_constant=lambda _: (_ for _ in ()).throw(ValueError('JSON constant')))
    if not isinstance(value, dict):
        raise ValueError('record is not an object')
    return value


def uint(value):
    if type(value) is not int or value < 0:
        raise ValueError('invalid nonnegative integer')
    return value


def hexbytes(value):
    if not isinstance(value, str) or not HEX.fullmatch(value):
        raise ValueError('invalid hex bytes')
    return len(value) // 2


def validate_value(v):
    if set(v) != {'kind', 'type', 'integer', 'float64_hex', 'string_hex', 'blob_hex'}:
        raise ValueError('invalid value fields')
    typ = uint(v['type'])
    if typ > 8 or not isinstance(v['integer'], str) or not re.fullmatch(r'-?(0|[1-9][0-9]*)', v['integer']):
        raise ValueError('invalid value type or integer')
    integer = int(v['integer'])
    if str(integer) != v['integer']:
        raise ValueError('noncanonical integer')
    if not -(1 << 63) <= integer < (1 << 63):
        raise ValueError('integer out of range')
    if hexbytes(v['float64_hex']) != 8:
        raise ValueError('invalid f64 bits')
    hexbytes(v['string_hex'])
    blob_len = hexbytes(v['blob_hex'])
    widths = {2: 1, 3: 4, 4: 8, 5: 4, 6: 8}
    if typ in widths and (integer < 0 or blob_len != integer * widths[typ]):
        raise ValueError('array length mismatch')
    if typ == 7:
        if integer < 0:
            raise ValueError('negative raw string count')
        data = v['blob_hex']; offset = 0
        for _ in range(integer):
            if offset + 16 > len(data):
                raise ValueError('truncated string array length')
            n = int(data[offset:offset + 16], 16); offset += 16
            if n > (len(data) - offset) // 2:
                raise ValueError('truncated string array bytes')
            offset += n * 2
        if offset != len(data):
            raise ValueError('trailing string array bytes')


def validate(stream, input_path=None, build_receipt=None):
    digest = hashlib.sha256(); count = 0; header = None; footer = None
    stack = []; values_left = 0; root_seen = False
    while True:
        line = stream.readline(MAX_LINE + 1)
        if not line:
            raise ValueError('missing footer')
        v = record(line); kind = v.get('kind')
        if header is None:
            if kind != 'header' or v.get('schema') != 'fbx-ufbx-dom/2':
                raise ValueError('invalid header')
            header = v
            for key in ('input_sha256', 'source_sha256', 'header_sha256', 'dumper_sha256', 'runner_sha256', 'build_receipt_sha256', 'executable_sha256'):
                if hexbytes(v.get(key)) != 32:
                    raise ValueError('invalid binding')
            uint(v.get('input_length'))
            if v.get('retain_dom') is not True or v.get('external_files') is not False or v.get('open_main_file_with_default') is not True or v.get('file_format') != 'FBX' or v.get('temp_allocator_bytes') != 805306368 or v.get('result_allocator_bytes') != 805306368:
                raise ValueError('unexpected oracle options')
            if input_path and (sha(input_path) != v['input_sha256'] or input_path.stat().st_size != v['input_length']):
                raise ValueError('input binding mismatch')
            if build_receipt:
                build = json.loads(build_receipt.read_text())
                bindings = {'source_sha256': build['profile']['ufbx.c'], 'header_sha256': build['profile']['ufbx.h'],
                            'dumper_sha256': sha(ROOT / 'reference/dom.c'), 'runner_sha256': sha(ROOT / 'reference/dom_dump.py'),
                            'build_receipt_sha256': sha(build_receipt), 'executable_sha256': build['executable_sha256']}
                if any(v.get(k) != value for k, value in bindings.items()):
                    raise ValueError('build binding mismatch')
            digest.update(line); continue
        if kind == 'footer':
            if values_left or any(stack) or not root_seen:
                raise ValueError('incomplete node tree')
            if set(v) != {'kind', 'records', 'sha256', 'oracle_exit'} or uint(v['records']) != count or uint(v['oracle_exit']) != 0 or v['sha256'] != digest.hexdigest():
                raise ValueError('invalid footer')
            if stream.read(1):
                raise ValueError('trailing data')
            footer = v; break
        if kind == 'node':
            if set(v) != {'kind', 'depth', 'name_hex', 'values', 'children'} or values_left:
                raise ValueError('invalid node or incomplete values')
            depth = uint(v['depth']); hexbytes(v['name_hex'])
            while stack and stack[-1] == 0:
                stack.pop()
            if depth != len(stack) or depth > MAX_DEPTH:
                raise ValueError('invalid depth')
            if not root_seen:
                if depth != 0:
                    raise ValueError('missing root')
                root_seen = True
            elif not stack:
                raise ValueError('multiple roots')
            if stack:
                stack[-1] -= 1
            values_left = uint(v['values']); stack.append(uint(v['children']))
        elif kind == 'value':
            if not values_left:
                raise ValueError('unexpected value')
            validate_value(v); values_left -= 1
        else:
            raise ValueError('unknown record kind')
        count += 1; digest.update(line)
    return {'records': count, 'input_sha256': header['input_sha256'], 'sha256': footer['sha256']}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('stream', type=Path); ap.add_argument('--input', type=Path); ap.add_argument('--build-receipt', type=Path)
    args = ap.parse_args()
    opener = gzip.open if args.stream.suffix == '.gz' else open
    with opener(args.stream, 'rb') as stream:
        result = validate(stream, args.input, args.build_receipt)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__': main()
