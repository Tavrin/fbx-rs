#!/usr/bin/env python3
"""Compare complete scenes, animation, deformers and typed list order.

Inputs are read in place. Receipts and streams go to a new external directory.
Retained numeric DOM arrays may differ only with exact oracle-side loaded-array
proofs. The documented file-axes versus target-axes distinction requires a
source-property proof and the fixed import target. No animation or list-order
exemptions apply.
"""
import argparse
import collections
import gzip
import hashlib
import json
import re
import subprocess
from pathlib import Path


def fields(data, expected_input=None):
    out, receipt = {}, None
    for line in data.splitlines():
        row = json.loads(line)
        if row['kind'] == 'receipt':
            if receipt is not None:
                raise ValueError('duplicate receipt')
            receipt = row
        if row['kind'] == 'field':
            path = row['path']
            if path in out:
                raise ValueError('duplicate field: ' + path)
            out[path] = row['value']
    if expected_input is not None and (receipt is None or receipt.get('input_sha256') != expected_input):
        raise ValueError('input hash binding differs')
    if not any(p.startswith('/order/') for p in out):
        raise ValueError('missing order section')
    return out


def flatten(value):
    """Flatten canonical scalar/vector chunks without changing any bits."""
    out = []
    for item in value:
        out.extend(flatten(item['vec3']) if 'vec3' in item else [item])
    return out


def mutation_proofs(oracle):
    """DOM ordinal -> exact post-load array; fail closed on missing owners."""
    names, ordinals, owners = {}, {}, {}
    for path, value in oracle.items():
        if path.startswith('/dom/') and path.endswith('/name'):
            names[path[:-5]] = bytes.fromhex(value['bytes_hex'])
        if path.startswith('/dom/') and path.endswith('/ordinal'):
            ordinals[path[:-8]] = value['u64']
        if path.startswith(('/meshes/', '/blend_shapes/')) and path.endswith('/element/dom_node') and value:
            prefix = path[:-len('/element/dom_node')]
            owners.setdefault(value['dom_ordinal'], []).append(prefix)
    proofs = {}
    for node, name in names.items():
        if name not in (b'Vertices', b'Normals', b'Indexes'):
            continue
        parent = node.rsplit('/children/', 1)[0]
        if names.get(parent) == b'LayerElementNormal':
            parent = parent.rsplit('/children/', 1)[0]
        for prefix in owners.get(ordinals.get(parent), []):
            if prefix.startswith('/meshes/'):
                attr = {b'Vertices': 'vertices', b'Normals': 'vertex_normal/values'}.get(name)
            else:
                attr = {b'Vertices': 'position_offsets', b'Normals': 'normal_offsets', b'Indexes': 'offset_vertices'}[name]
            if attr is None:
                continue
            loaded, dom = [], []
            for path, value in oracle.items():
                if re.fullmatch(re.escape(prefix + '/' + attr) + r'/\d+', path):
                    loaded.extend(flatten(value))
                if re.fullmatch(re.escape(node) + r'/values/0/array/\d+', path):
                    dom.extend(value)
            def scalar(value):
                for kind in ('i64', 'u64', 'i32', 'u32'):
                    if kind in value:
                        return ('integer', int(value[kind]))
                return value
            if loaded and [scalar(v) for v in loaded] == [scalar(v) for v in dom]:
                proofs[node] = {'owner': prefix, 'attribute': attr, 'values': len(loaded)}
    return proofs


def axes_proof(oracle, rust):
    """ufbxi_update_scene_settings retains source axes; the public scene reports Y-up.

    Admit only the three axis enums, with oracle values reconstructed from
    identical authored settings properties and Rust values equal to the pinned
    right-handed Y-up target. Missing/invalid/duplicate evidence fails closed.
    """
    props = {}
    prefix = '/globals/settings/props/props/'
    for path, value in oracle.items():
        if not re.fullmatch(re.escape(prefix) + r'\d+/name', path):
            continue
        name = value.get('bytes_hex') if isinstance(value, dict) else None
        if name is None or path not in rust or rust[path] != value:
            return None
        val_path = path[:-4] + 'value_int'
        val = oracle.get(val_path)
        if not isinstance(val, dict) or rust.get(val_path) != val:
            return None
        if name in props:
            return None
        props[name] = val.get('i64')
    source = {}
    for axis, prop, target in [('right', 'CoordAxis', 0), ('up', 'UpAxis', 2), ('front', 'FrontAxis', 4)]:
        dim = props.get(prop.encode().hex())
        sign = props.get((prop + 'Sign').encode().hex())
        if dim not in ('0', '1', '2') or sign not in ('-1', '1'):
            return None
        source[axis] = int(dim) * 2 + int(sign == '-1')
        path = '/globals/settings/axes/' + axis
        if oracle.get(path) != {'enum': 'ufbx_coordinate_axis', 'i64': str(source[axis])}:
            return None
        if rust.get(path) != {'enum': 'ufbx_coordinate_axis', 'i64': str(target)}:
            return None
    if len({v // 2 for v in source.values()}) != 3:
        return None
    return {'source': source, 'target': {'right': 0, 'up': 2, 'front': 4}}


def compare(oracle, rust):
    differences = sorted(p for p in oracle.keys() | rust.keys() if oracle.get(p) != rust.get(p))
    proofs = mutation_proofs(oracle) if any(p.startswith('/dom/') for p in differences) else {}
    axes = axes_proof(oracle, rust) if any(p.startswith('/globals/settings/axes/') for p in differences) else None
    explained, explained_axes, unexplained = [], [], []
    for path in differences:
        match = re.fullmatch(r'(.*)/values/0/array/\d+', path)
        node = match.group(1) if match else None
        if node in proofs and path in oracle and path in rust:
            explained.append(path)
        elif axes and path in ('/globals/settings/axes/right', '/globals/settings/axes/up', '/globals/settings/axes/front'):
            explained_axes.append(path)
        else:
            unexplained.append(path)
    return {
        'status': 'equal' if not unexplained else 'differs',
        'order_equal': all(oracle.get(p) == rust.get(p) for p in oracle.keys() | rust.keys() if p.startswith('/order/')),
        'explained_dom_fields': len(explained),
        'mutation_proofs': proofs,
        'explained_axes_fields': len(explained_axes),
        'axes_proof': axes,
        'unexplained_fields': len(unexplained),
        'first_differences': [{'path': p, 'oracle': oracle.get(p), 'rust': rust.get(p)} for p in unexplained[:20]],
    }


def run(command, destination):
    with destination.open('xb') as out, Path(str(destination) + '.stderr').open('xb') as err:
        try:
            result = subprocess.run(command, stdout=out, stderr=err, timeout=120, check=False)
            return result.returncode
        except subprocess.TimeoutExpired:
            return 124


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', type=Path)
    parser.add_argument('oracle', type=Path)
    parser.add_argument('parity', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    results, cache = [], {}
    for path in args.paths.read_text().splitlines():
        if not path:
            continue
        with Path(path).open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        if digest not in cache:
            oracle, rust = args.out / (digest + '.oracle.jsonl'), args.out / (digest + '.rust.jsonl')
            code_o = run([str(args.oracle), path], oracle)
            code_r = run([str(args.parity), 'dump-scene', path], rust)
            row = {'oracle_exit': code_o, 'rust_exit': code_r}
            if code_o or code_r:
                row['status'] = 'error'
                row['stderr'] = Path(str(rust) + '.stderr').read_text(errors='replace')[:1000]
            else:
                valid_o = run([str(args.parity), 'validate', str(oracle)], args.out / (digest + '.validate-oracle'))
                valid_r = run([str(args.parity), 'validate', str(rust)], args.out / (digest + '.validate-rust'))
                row['validation_exits'] = [valid_o, valid_r]
                if valid_o or valid_r:
                    row['status'] = 'invalid'
                else:
                    try:
                        a, b = fields(oracle.read_bytes(), digest), fields(rust.read_bytes(), digest)
                        row.update(compare(a, b))
                        row['version'] = a.get('/globals/metadata/version')
                    except ValueError as error:
                        row.update(status='invalid', reason=str(error))
                row['public_exit'] = run([str(args.parity), 'check-public', path], args.out / (digest + '.public'))
                if row['public_exit']:
                    row['status'] = 'error'
            for stream in (oracle, rust):
                with stream.open('rb') as source, gzip.open(str(stream) + '.gz', 'wb') as dest:
                    while chunk := source.read(1 << 20):
                        dest.write(chunk)
                stream.unlink()
            cache[digest] = row
        result = {'path': path, 'sha256': digest, **cache[digest]}
        results.append(result)
        (args.out / 'results.json').write_text(json.dumps({'results': results}, indent=2) + '\n')
        print(Path(path).name, result['status'], flush=True)
    counts = dict(collections.Counter(r['status'] for r in results))
    report = {'counts': counts, 'distinct_files': len(cache), 'results': results}
    (args.out / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'counts': counts, 'distinct_files': len(cache)}))
    return int(not results or any(r['status'] != 'equal' for r in results))


if __name__ == '__main__':
    raise SystemExit(main())
