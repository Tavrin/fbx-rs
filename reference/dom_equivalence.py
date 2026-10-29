#!/usr/bin/env python3
"""Corpus DOM equivalence: Rust syntax DOM vs the pinned ufbx retained DOM.

Inputs per file: the Rust stream (`fbx-syntax-dump/2`, parity/examples/dump_syntax.rs)
and the oracle stream (`fbx-ufbx-dom/2`, reference/dom.c + dom_dump.py).

The Rust DOM keeps source property types; ufbx's retained DOM does not. The
projection below rewrites every Rust value into the value ufbx retains, using
only rules taken from the pinned ufbx source (C 0.11.0 at
5d61cfce6272523f7a51b9d8580e156b352e1247; line numbers refer to its ufbx.c):

* scalars (9877-9956, 8486-8587, 9626-9720): numbers become NUMBER with an
  int64 and a double view; C/B read as signed char; F/D give the int view via
  ufbxi_f64_to_i64 (854-870); ASCII integer `-0` keeps a -0.0 double view.
* strings (6977-7020, 4464-4600): blob = source bytes; the string view is the
  bytes for printable-ASCII, else "" (raw context) or the UTF-8-sanitised bytes.
  The context is ufbx's parse state, so the view is accepted when it equals one
  of the two values ufbx can produce for those bytes.
* at most 8 scalar values per node (UFBXI_MAX_NON_ARRAY_VALUES); in binary a
  non-array node's values stop at the first array (mask 0 ends the DOM loop);
  in ASCII an `*N {a: ...}` array in a non-array node contributes its elements
  as scalars (9722-9758).
* array nodes (ufbx chooses them by parse context, 7323-7735; the oracle's
  value type tells which one it chose): elements converted to the oracle's
  element type with ufbxi_binary_convert_array (7903-7995), the multivalue
  reader (7998-8104) or the ASCII value rules (9626-9720); boolean arrays
  normalised to 0/1 (8128-8133); ignored arrays retain nothing (8467-8471).

A file is `equal` when every projected value equals the oracle bit for bit.
Differences the projection cannot produce are `classified` only when an
evidenced ufbx post-parse mutation explains them exactly:

* oracle_blend_offset_sort: ufbxi_read_shape (12122-12182) stable-sorts a
  shape's Indexes in place and permutes Vertices/Normals triples with them;
  the retained DOM shares those arrays.

Everything else is `failing`.
"""
import argparse
import collections
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import subprocess
import sys

I64_MIN, I64_MAX = -(1 << 63), (1 << 63) - 1
I32_MIN, I32_MAX = -(1 << 31), (1 << 31) - 1
ARRAY_KINDS = 'bcilfd'
TARGET = {2: 'c', 3: 'i', 4: 'l', 5: 'f', 6: 'd'}
BOOL_ARRAYS = {b'Smoothing', b'Visibility', b'Hole'}  # ufbx array type 'b' (7568-7598)


def f64hex(x):
    return struct.pack('>d', x).hex()


def f64_to_i64(x):  # ufbxi_f64_to_i64; x86-64 cvttsd2si for the 2^63 edge
    if math.isnan(x):
        return I64_MIN
    if abs(x) <= 9223372036854775807.0:
        return I64_MIN if x >= 9223372036854775808.0 else int(x)
    return I64_MAX if x >= 0 else I64_MIN


def f64_to_i32(x):  # ufbxi_f64_to_i32
    if math.isnan(x):
        return I32_MIN
    if abs(x) <= 2147483647.0:
        return int(x)
    return I32_MAX if x >= 0 else I32_MIN


def to_u8(x):  # C (uint8_t) of a double on x86-64: cvttsd2si to int32, low byte
    if math.isnan(x) or abs(x) >= 2147483648.0:
        return 0
    return int(x) & 0xff


def f32(x):  # C (float) of a double: round to nearest, overflow to infinity
    try:
        return struct.unpack('>f', struct.pack('>f', x))[0]
    except OverflowError:
        return math.copysign(math.inf, x)


def wrap(v, bits):
    v &= (1 << bits) - 1
    return v - (1 << bits) if v >> (bits - 1) else v


def encode(target, values):
    if target == 'c':
        return bytes(v & 0xff for v in values).hex()
    fmt = {'i': 'i', 'l': 'q', 'f': 'f', 'd': 'd'}[target]
    return struct.pack('>%d%s' % (len(values), fmt), *values).hex()


def number(i, f):
    return (0, str(i), f64hex(f), '', '')


# Elements: ('int', v) exact integer token/value; ('float', x) double;
# ('negzero',) ASCII integer -0; ('word', c) ASCII bare word first byte.
def ascii_tokens(value):
    t = value['t']
    if t == 'L':
        return [('int', int(value['i']))]
    if t == 'Z':
        return [('negzero',)]
    if t == 'D':
        return [('float', struct.unpack('>d', bytes.fromhex(value['x']))[0])]
    if t == 'W':
        word = bytes.fromhex(value['x'])
        return [('word', wrap(word[0], 8) if word else 0)]
    if t == 'A':
        out = []
        for item in value['v'].split(',') if value['v'] else []:
            tag, rest = item[0], item[1:]
            if tag == 'i':
                out.append(('int', int(rest)))
            elif tag == 'f':
                out.append(('float', struct.unpack('>d', bytes.fromhex(rest))[0]))
            elif tag == 'w':
                word = bytes.fromhex(rest)
                out.append(('word', wrap(word[0], 8) if word else 0))
            elif tag == 'z':
                out.append(('negzero',))
            else:
                raise ValueError('bad ascii array element')
        return out
    raise ValueError('unexpected ASCII value type ' + t)


def scalar_from_token(tok):
    if tok[0] in ('int', 'word'):
        return number(tok[1], float(tok[1]))
    if tok[0] == 'negzero':
        return number(0, -0.0)
    return number(f64_to_i64(tok[1]), tok[1])


def binary_scalar(value):
    t = value['t']
    if t in ('C', 'B'):
        v = wrap(int(value['i']), 8)
        return number(v, float(v))
    if t in ('Y', 'I', 'L'):
        v = int(value['i'])
        return number(v, float(v))
    if t == 'F':
        x = struct.unpack('>f', bytes.fromhex(value['x']))[0]
        return (0, str(f64_to_i64(x)), f64hex(x), '', '')
    if t == 'D':
        x = struct.unpack('>d', bytes.fromhex(value['x']))[0]
        return (0, str(f64_to_i64(x)), value['x'], '', '')
    raise ValueError('unexpected binary scalar ' + t)


def utf8_valid_length(b):  # ufbxi_utf8_valid_length (2895)
    i, n = 0, len(b)
    while i < n:
        step = utf8_step(b, i)
        if not step:
            break
        i += step
    return i


def utf8_step(b, i):
    c, left = b[i], len(b) - i
    if c & 0x80 == 0:
        return 1 if c else 0
    if c & 0xe0 == 0xc0 and left >= 2:
        code = c << 8 | b[i + 1]
        return 2 if (code & 0xc0) == 0x80 and code >= 0xc280 else 0
    if c & 0xf0 == 0xe0 and left >= 3:
        code = c << 16 | b[i + 1] << 8 | b[i + 2]
        return 3 if (code & 0xc0c0) == 0x8080 and code >= 0xe0a080 and (code < 0xeda080 or code >= 0xee8080) else 0
    if c & 0xf8 == 0xf0 and left >= 4:
        code = c << 24 | b[i + 1] << 16 | b[i + 2] << 8 | b[i + 3]
        return 4 if (code & 0xc0c0c0) == 0x808080 and 0xf0908080 <= code <= 0xf48fbfbf else 0
    return 0


def sanitize(b):  # ufbxi_sanitize_string with REPLACEMENT_CHARACTER (4464)
    out, i = bytearray(), 0
    while i < len(b):
        step = utf8_step(b, i)
        if step:
            out += b[i:i + step]
            i += step
        else:
            out += b'\xef\xbf\xbd'
            i += 1
    return bytes(out)


def string_value(raw):
    """Projected STRING plus the allowed string views and the rule used."""
    if all(0 < c < 0x80 for c in raw):
        views = {raw.hex(): 'string_view_ascii'}
    elif utf8_valid_length(raw) == len(raw):
        views = {raw.hex(): 'string_view_utf8', '': 'string_view_raw_context_empty'}
    else:
        views = {sanitize(raw).hex(): 'string_view_sanitized', '': 'string_view_raw_context_empty'}
    return (1, '0', '0' * 16, views, raw.hex())


def binary_array_elements(value):
    """(normalised kind, python values, big-endian hex) of a binary array."""
    kind, n, data = value['k'], value['n'], bytes.fromhex(value['x'])
    if kind in 'bc':
        return 'c', [wrap(b, 8) for b in data], value['x']
    fmt = {'i': 'i', 'l': 'q', 'f': 'f', 'd': 'd'}[kind]
    return kind, list(struct.unpack('>%d%s' % (n, fmt), data)), value['x']


def convert_binary(src, target, vals):  # ufbxi_binary_convert_array
    if target == 'c':
        return [v & 0xff if src in 'cil' else to_u8(v) for v in vals]
    if target == 'i':
        return [wrap(v, 32) if src in 'cl' else f64_to_i32(v) for v in vals]
    if target == 'l':
        return [v if src in 'ci' else f64_to_i64(v) for v in vals]
    if target == 'f':
        return [f32(float(v)) if src in 'cil' else f32(v) for v in vals]
    return [float(v) for v in vals]


def convert_scalar_int(target, v):  # m_cast_int of ufbxi_binary_parse_multivalue_array
    return {'c': v & 0xff, 'i': wrap(v, 32), 'l': wrap(v, 64), 'f': f32(float(v)), 'd': float(v)}[target]


def convert_scalar_float(target, x):  # m_cast_float
    return {'c': to_u8(x), 'i': f64_to_i32(x), 'l': f64_to_i64(x), 'f': f32(x), 'd': x}[target]


def convert_ascii(target, tok, boolean, accurate_f32):
    kind = tok[0]
    if boolean:
        return 0 if kind == 'negzero' else int(tok[1] != 0)
    if kind == 'negzero':
        return {'c': 0, 'i': 0, 'l': 0, 'f': -0.0, 'd': -0.0}[target]
    if kind in ('int', 'word'):
        return convert_scalar_int(target, tok[1])
    x = tok[1]
    if accurate_f32 and target == 'i':
        x = f32(x)  # KeyAttrDataFloat in ASCII >= 7200 is read with strtof (9036, 7420-7426)
    return convert_scalar_float(target, x)


def array_value(target_type, elems_hex, count):
    return (target_type, str(count), f64hex(float(count)), '', elems_hex)


def project_array(fmt, version, name, values, target_type, used):
    if target_type == 8:  # UFBX_DOM_VALUE_ARRAY_IGNORED
        used['ignored_array'] += 1
        return [array_value(8, '', 0)]
    if target_type == 7:  # raw strings: video Content / legacy Children
        strings = [bytes.fromhex(v['x']) for v in values if v['t'] in ('S', 'R')]
        blob = ''.join('%016x' % len(s) + s.hex() for s in strings)
        return [array_value(7, blob, len(strings))]
    target = TARGET[target_type]
    boolean = target == 'c' and name in BOOL_ARRAYS
    if fmt == 'binary':
        if not values:
            return [array_value(target_type, '', 0)]
        first = values[0]
        if first['t'] == 'a':
            if len(values) > 1:
                used['values_after_array_dropped'] += 1
            src, vals, raw_hex = binary_array_elements(first)
            if src == target and not boolean:
                return [array_value(target_type, raw_hex, len(vals))]
            out = vals if src == target else convert_binary(src, target, vals)
            used['array_converted'] += src != target
        else:
            out = []
            for v in values:
                if v['t'] in ('C', 'B', 'Y', 'I', 'L'):
                    iv = wrap(int(v['i']), 8) if v['t'] in ('C', 'B') else int(v['i'])
                    out.append(convert_scalar_int(target, iv))
                elif v['t'] in ('F', 'D'):
                    x = struct.unpack('>f' if v['t'] == 'F' else '>d', bytes.fromhex(v['x']))[0]
                    out.append(convert_scalar_float(target, x))
                else:
                    raise ValueError('non-numeric value in a multivalue array')
            used['multivalue_array'] += 1
        if boolean:
            out = [int(v != 0) for v in out]
            used['bool_normalised'] += 1
        return [array_value(target_type, encode(target, out), len(out))]
    accurate = name == b'KeyAttrDataFloat' and version >= 7200
    toks = []
    for v in values:
        if v['t'] == 'S':
            continue  # strings in numeric arrays are ignored (9600-9603)
        toks.extend(ascii_tokens(v))
    out = [convert_ascii(target, t, boolean, accurate) for t in toks]
    if boolean:
        used['bool_normalised'] += 1
    return [array_value(target_type, encode(target, out), len(out))]


def project_scalars(fmt, values, used):
    out = []
    if fmt == 'binary':
        for i, v in enumerate(values):
            if v['t'] == 'a':
                used['values_after_array_dropped' if i + 1 < len(values) else 'array_in_scalar_node'] += 1
                break
            out.append(string_value(bytes.fromhex(v['x'])) if v['t'] in ('S', 'R') else binary_scalar(v))
    else:
        for v in values:
            if v['t'] == 'S':
                out.append(string_value(bytes.fromhex(v['x'])))
            else:
                if v['t'] == 'A':
                    used['ascii_array_in_scalar_node'] += 1
                out.extend(scalar_from_token(t) for t in ascii_tokens(v))
    if len(out) > 8:
        used['values_capped_at_8'] += 1
        out = out[:8]
    return out


def project(fmt, version, name, values, oracle_values, used):
    if len(oracle_values) == 1 and oracle_values[0]['type'] >= 2:
        return project_array(fmt, version, name, values, oracle_values[0]['type'], used)
    return project_scalars(fmt, values, used)


def oracle_tuple(o):
    return (o['type'], o['integer'], o['float64_hex'], o['string_hex'], o['blob_hex'])


def values_equal(projected, oracle, used):
    if len(projected) != len(oracle):
        return False
    rules = []
    for p, o in zip(projected, oracle):
        o = oracle_tuple(o)
        if isinstance(p[3], dict):
            if p[0] != o[0] or p[1] != o[1] or p[2] != o[2] or p[4] != o[4] or o[3] not in p[3]:
                return False
            rules.append(p[3][o[3]])
        elif p != o:
            return False
    for r in rules:
        used[r] += 1
    return True


def read_oracle(path):
    nodes, current, rolling, footer = [], None, hashlib.sha256(), None
    with gzip.open(path, 'rb') as stream:
        for line in stream:
            x = json.loads(line)
            if x['kind'] == 'footer':
                footer = x
                break
            rolling.update(line)
            if x['kind'] == 'node':
                current = {'depth': x['depth'] - 1, 'name': bytes.fromhex(x['name_hex']), 'values': []}
                if x['depth'] > 0:
                    nodes.append(current)
            elif x['kind'] == 'value':
                current['values'].append(x)
    if not footer or footer['sha256'] != rolling.hexdigest():
        raise ValueError('oracle checksum mismatch')
    return nodes


def blend_sort(rust, oracle, projected, parents, failing):
    """Return the failing node indices explained by ufbxi_read_shape's sort."""
    explained = set()
    groups = collections.defaultdict(dict)
    for i in failing:
        if rust[i]['name'] in (b'Indexes', b'Vertices', b'Normals'):
            groups[parents[i]][rust[i]['name']] = i
    for parent, members in groups.items():
        siblings = {rust[j]['name']: j for j in range(len(rust)) if parents[j] == parent
                    and rust[j]['name'] in (b'Indexes', b'Vertices', b'Normals')}
        if b'Indexes' not in siblings or b'Vertices' not in siblings:
            continue
        idx = projected[siblings[b'Indexes']]
        if len(idx) != 1 or idx[0][0] != 3:
            continue
        indices = list(struct.unpack('>%di' % int(idx[0][1]), bytes.fromhex(idx[0][4])))
        order = sorted(range(len(indices)), key=lambda n: indices[n] & 0xffffffff)
        ok = True
        for nm, j in siblings.items():
            p = projected[j]
            if len(p) != 1 or p[0][0] < 2 or p[0][0] > 6:
                ok = False
                break
            width = {2: 1, 3: 4, 4: 8, 5: 4, 6: 8}[p[0][0]] * 2
            per = 1 if nm == b'Indexes' else 3
            elems = [p[0][4][k:k + width] for k in range(0, len(p[0][4]), width)]
            if len(elems) != per * len(indices):
                ok = False
                break
            permuted = ''.join(elems[per * n + k] for n in order for k in range(per))
            if (p[0][0], p[0][1], p[0][2], p[0][3], permuted) != oracle_tuple(oracle[j]['values'][0]):
                ok = False
                break
        if ok:
            explained.update(siblings.values())
    return explained


def compare(rust_header, rust, oracle):
    used = collections.Counter()
    if len(rust) != len(oracle):
        return {'verdict': 'failing', 'reason': 'node count %d vs %d' % (len(rust), len(oracle))}
    fmt, version = rust_header['format'], rust_header['version']
    parents, stack, projected, failing = [], [], [], []
    for i, (r, o) in enumerate(zip(rust, oracle)):
        if r['depth'] != o['depth'] or r['name'] != o['name']:
            return {'verdict': 'failing', 'reason': 'structure differs at node %d' % i}
        del stack[r['depth']:]
        parents.append(stack[-1] if stack else None)
        stack.append(i)
        try:
            p = project(fmt, version, r['name'], r['values'], o['values'], used)
        except (ValueError, KeyError, struct.error) as e:
            p = [('projection error', str(e))]
        projected.append(p)
        if not values_equal(p, o['values'], used):
            failing.append(i)
    classified = collections.Counter()
    if failing:
        explained = blend_sort(rust, oracle, projected, parents, failing)
        if explained:
            classified['oracle_blend_offset_sort'] += len(explained & set(failing))
        failing = [i for i in failing if i not in explained]
    result = {'nodes': len(rust), 'projection_rules': dict(used), 'classified': dict(classified)}
    if failing:
        i = failing[0]
        result.update(verdict='failing', failing_nodes=len(failing), first={
            'index': i, 'name': rust[i]['name'].decode('latin1'),
            'rust': [str(x)[:300] for x in rust[i]['values']][:4],
            'projected': [str(x)[:300] for x in projected[i]][:4],
            'oracle': [str(oracle_tuple(x))[:300] for x in oracle[i]['values']][:4]})
    else:
        result['verdict'] = 'classified' if classified else 'equal'
    return result


def run_file(exe, path, oracle_path):
    proc = subprocess.run([exe, path], capture_output=True, timeout=600)
    if proc.returncode:
        return None, proc.stderr.decode('utf8', 'replace').strip()
    lines = proc.stdout.splitlines()
    header = json.loads(lines[0])
    if header.get('schema') != 'fbx-syntax-dump/2':
        raise ValueError('bad rust stream header')
    rust = []
    for line in lines[1:]:
        x = json.loads(line)
        rust.append({'depth': x['depth'], 'name': bytes.fromhex(x['name_hex']), 'values': x['values']})
    return compare(header, rust, read_oracle(oracle_path)), None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--manifest', type=Path, required=True)
    ap.add_argument('--oracle-dir', type=Path, required=True)
    ap.add_argument('--exe', required=True)
    ap.add_argument('--out', type=Path, required=True)
    args = ap.parse_args()
    manifest = {}
    for line in args.manifest.open():
        row = json.loads(line)
        manifest.setdefault(row['sha256'], row)
    results = []
    for digest, row in sorted(manifest.items()):
        with open(row['path'], 'rb') as f:
            data = f.read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise SystemExit('input hash mismatch ' + digest)
        version = header_version(data)
        entry = {'sha256': digest, 'header_version': version}
        oracle_path = args.oracle_dir / (digest + '.jsonl.gz')
        result, error = run_file(args.exe, row['path'], oracle_path)
        if result is None:
            supported = version is not None and 7100 <= version <= 7700
            entry.update(verdict='failing' if supported or not error.startswith('UnsupportedVersion')
                         else 'rejected_unsupported', rust_error=error)
        else:
            entry.update(result)
        results.append(entry)
    totals = collections.Counter(r['verdict'] for r in results)
    rules = collections.Counter()
    for r in results:
        rules.update(r.get('projection_rules', {}))
        rules.update(r.get('classified', {}))
    summary = {'files': len(results), 'verdicts': dict(totals), 'rules': dict(rules),
               'exe_sha256': hashlib.sha256(Path(args.exe).read_bytes()).hexdigest(),
               'comparator_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    args.out.write_text(json.dumps({'summary': summary, 'files': results}, indent=1) + '\n')
    print(json.dumps(summary), flush=True)
    return 0 if totals.get('failing', 0) == 0 else 1


def header_version(data):
    if data.startswith(b'Kaydara FBX Binary  \x00\x1a') and len(data) >= 27:
        return int.from_bytes(data[23:27], 'big' if data[22] else 'little')
    m = re.search(rb'FBX (\d)\.(\d)\.(\d)', data[:256])
    if m:
        return int(m.group(1)) * 1000 + int(m.group(2)) * 100 + int(m.group(3)) * 10
    m = re.search(rb'FBXVersion:\s*(\d+)', data)
    return int(m.group(1)) if m else None


if __name__ == '__main__':
    sys.exit(main())
