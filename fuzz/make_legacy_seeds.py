#!/usr/bin/env python3
"""Emit self-authored legacy ASCII and endian/mixed-Key binary fuzz seeds.

Usage: python3 fuzz/make_legacy_seeds.py OUTPUT
Add these files to the persistent seed corpus of binary/ascii/scene_dom/load.
This generates seeds only; it does not run fuzzing.
"""
import argparse
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def binary(be, legacy_root=False):
    endian = '>' if be else '<'

    def prop(value):
        if isinstance(value, bytes):
            return b'S' + struct.pack(endian + 'I', len(value)) + value
        if isinstance(value, int):
            return b'L' + struct.pack(endian + 'q', value)
        if isinstance(value, float):
            return b'D' + struct.pack(endian + 'd', value)
        kind, data = value
        if kind == 'char':
            return b'C' + data
        raw = b''.join(struct.pack(endian + kind, x) for x in data)
        compressed = zlib.compress(raw)
        return kind.encode() + struct.pack(endian + 'III', len(data), 1, len(compressed)) + compressed

    def node(offset, name, values=(), children=()):
        name = name.encode()
        properties = b''.join(prop(p) for p in values)
        cursor = offset + 13 + len(name) + len(properties)
        body = b''
        for child in children:
            encoded = node(cursor, *child)
            body += encoded
            cursor += len(encoded)
        if children:
            body += bytes(13)
            cursor += 13
        return struct.pack(endian + 'IIIB', cursor, len(values), len(properties), len(name)) + name + properties + body

    def channel(name, default, keys=None):
        children = [('Default', [default])]
        if keys is not None:
            children += [('KeyVer', [4005]), ('KeyCount', [2]), ('Key', keys)]
        return ('Channel', [name], children)

    model = b'Child\x00\x01Model'
    raw = b'Kaydara FBX Binary  \x00\x1a' + bytes([int(be)]) + struct.pack(endian + 'I', 3000 if legacy_root else 6100)
    roots = [
        ('FBXHeaderExtension', [], [('FBXVersion', [6100])]),
        ('Objects', [], [
            ('Model', [model, b'Mesh'], [
                ('Properties60', [], [
                    ('Property', [b'Lcl Translation', b'Lcl Translation', b'A', 0.0, 2.0, 3.0]),
                ]),
                ('Vertices', [('d', [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])]),
                ('PolygonVertexIndex', [('i', [0, 1, -3])]),
            ]),
        ]),
        ('Connections', [], [('Connect', [b'OO', model, b'Scene\x00\x01Model'])]),
        ('Takes', [], [('Take', [b'Walk'], [
            ('LocalTime', [0, 46186158000]),
            ('Model', [model], [('Channel', [b'Transform'], [
                ('Channel', [b'T'], [
                    channel(b'X', 0.0, [0, 0.0, ('char', b'L'), 46186158000, 10.0, ('char', b'L')]),
                    channel(b'Y', 2.0), channel(b'Z', 3.0),
                ]),
            ])]),
        ])]),
    ]
    if legacy_root:
        roots = [
            ('FBXHeaderExtension', [], [('FBXVersion', [5800])]),
            ('Model', [model], [
                ('Vertices', [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
                ('PolygonVertexIndex', [0, 1, -3]),
                ('MaterialAssignation', [b'AllSame']), ('Materials', [0]),
            ]),
            roots[-1],
        ]
    for entry in roots:
        raw += node(len(raw), *entry)
    return raw + bytes(13)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for name in ['legacy-take.fbx', 'legacy-root.fbx', 'legacy-deform.fbx', 'legacy-texture.fbx']:
        (args.output / name).write_bytes((ROOT / 'tests/fixtures' / name).read_bytes())
    (args.output / 'legacy-v5-binary.fbx').write_bytes(binary(False, legacy_root=True))
    for be in [False, True]:
        (args.output / ('legacy-be.fbx' if be else 'legacy-le.fbx')).write_bytes(binary(be))


if __name__ == '__main__':
    main()
