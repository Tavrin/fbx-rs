#!/usr/bin/env python3
"""Fault tests for the corpus DOM equivalence comparator."""
import struct
import unittest

from dom_equivalence import compare


def oracle_value(typ, integer, f64, string='', blob=''):
    return {'type': typ, 'integer': str(integer), 'float64_hex': f64, 'string_hex': string, 'blob_hex': blob}


def num(i, f):
    return oracle_value(0, i, struct.pack('>d', f).hex())


def node(name, values, depth=0):
    return {'depth': depth, 'name': name, 'values': values}


def verdict(fmt, rust_nodes, oracle_nodes, version=7400):
    return compare({'format': fmt, 'version': version}, rust_nodes, oracle_nodes)['verdict']


class EquivalenceTest(unittest.TestCase):
    def test_binary_scalars_fold_and_bit_fault(self):
        rust = [node(b'N', [{'t': 'C', 'i': '255'}, {'t': 'F', 'x': '3fc00000'}, {'t': 'L', 'i': '-3'}])]
        oracle = [node(b'N', [num(-1, -1.0), num(1, 1.5), num(-3, -3.0)])]
        self.assertEqual(verdict('binary', rust, oracle), 'equal')
        oracle[0]['values'][1] = num(1, 1.5000000000000002)
        self.assertEqual(verdict('binary', rust, oracle), 'failing')

    def test_string_views(self):
        raw = b'Name\x00\x01Model'
        rust = [node(b'C', [{'t': 'S', 'x': raw.hex()}])]
        sanitized = b'Name\xef\xbf\xbd\x01Model'
        for view, expected in [(sanitized, 'equal'), (b'', 'equal'), (raw, 'failing')]:
            oracle = [node(b'C', [oracle_value(1, 0, '0' * 16, view.hex(), raw.hex())])]
            self.assertEqual(verdict('binary', rust, oracle), expected, view)
        oracle = [node(b'C', [oracle_value(1, 0, '0' * 16, '', b'Name\x00\x01Modem'.hex())])]
        self.assertEqual(verdict('binary', rust, oracle), 'failing')

    def test_array_conversion_and_element_fault(self):
        rust = [node(b'Vertices', [{'t': 'a', 'k': 'f', 'n': 2, 'x': '3f80000040000000'}])]
        blob = struct.pack('>2d', 1.0, 2.0).hex()
        oracle = [node(b'Vertices', [oracle_value(6, 2, struct.pack('>d', 2.0).hex(), '', blob)])]
        self.assertEqual(verdict('binary', rust, oracle), 'equal')
        oracle[0]['values'][0]['blob_hex'] = blob[:-1] + '1'
        self.assertEqual(verdict('binary', rust, oracle), 'failing')

    def test_ascii_value_cap_negative_zero_and_count_fault(self):
        rust = [node(b'P', [{'t': 'Z'}, {'t': 'A', 'v': ','.join('i%d' % n for n in range(9))}])]
        oracle = [node(b'P', [num(0, -0.0)] + [num(n, float(n)) for n in range(7)])]
        self.assertEqual(verdict('ascii', rust, oracle), 'equal')
        oracle[0]['values'].append(num(7, 7.0))
        self.assertEqual(verdict('ascii', rust, oracle), 'failing')

    def test_structure_fault(self):
        rust = [node(b'A', []), node(b'B', [], 1)]
        self.assertEqual(verdict('binary', rust, [node(b'A', []), node(b'B', [], 0)]), 'failing')
        self.assertEqual(verdict('binary', rust, [node(b'A', []), node(b'C', [], 1)]), 'failing')
        self.assertEqual(verdict('binary', rust, [node(b'A', [])]), 'failing')

    def test_blend_offset_sort_is_classified_only_when_exact(self):
        def shape(indices, vertices):
            return [node(b'Shape', []),
                    node(b'Indexes', [{'t': 'A', 'v': ','.join('i%d' % i for i in indices)}], 1),
                    node(b'Vertices', [{'t': 'A', 'v': ','.join('i%d' % v for v in vertices)}], 1)]

        def oracle(indices, vertices):
            idx = struct.pack('>%di' % len(indices), *indices).hex()
            ver = struct.pack('>%dd' % len(vertices), *vertices).hex()
            return [node(b'Shape', []),
                    node(b'Indexes', [oracle_value(3, len(indices), struct.pack('>d', len(indices)).hex(), '', idx)], 1),
                    node(b'Vertices', [oracle_value(6, len(vertices), struct.pack('>d', len(vertices)).hex(), '', ver)], 1)]

        rust = shape([1, 0], [1, 1, 1, 0, 0, 0])
        self.assertEqual(verdict('ascii', rust, oracle([0, 1], [0, 0, 0, 1, 1, 1])), 'classified')
        self.assertEqual(verdict('ascii', rust, oracle([0, 1], [1, 1, 1, 0, 0, 0])), 'failing')


if __name__ == '__main__':
    unittest.main()
