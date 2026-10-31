#!/usr/bin/env python3
"""Fault tests for strict corpus comparison, independent of scene loading."""
import copy
import unittest
from compare_corpus import compare, fields


class CorpusComparison(unittest.TestCase):
    def test_animation_and_order_are_never_exempt(self):
        oracle = {'/order/1/0': {'ref': 'root'}, '/curves/example/keyframes/0/value': {'f64': '0000000000000000'}}
        for path in oracle:
            rust = copy.deepcopy(oracle)
            rust[path] = None
            result = compare(oracle, rust)
            self.assertEqual(result['status'], 'differs')
            self.assertEqual(result['unexplained_fields'], 1)

    def test_proof_is_bound_to_dom_ordinal_and_exact_loaded_bits(self):
        vector = [{'f64': '3ff0000000000000'}, {'f64': '0000000000000000'}, {'f64': '0000000000000000'}]
        node = '/dom/children/0/children/0'
        oracle = {
            node + '/ordinal': {'u64': '2'},
            node + '/name': {'bytes_hex': b'Model'.hex()},
            node + '/children/0/name': {'bytes_hex': b'Vertices'.hex()},
            node + '/children/0/values/0/array/0': vector,
            '/meshes/example/element/dom_node': {'dom_ordinal': '2'},
            '/meshes/example/vertices/0': [{'vec3': copy.deepcopy(vector)}],
        }
        rust = copy.deepcopy(oracle)
        rust[node + '/children/0/values/0/array/0'][0]['f64'] = '4000000000000000'
        self.assertEqual(compare(oracle, rust)['status'], 'equal')
        for change in ['ordinal', 'bits']:
            corrupted = copy.deepcopy(oracle)
            if change == 'ordinal':
                corrupted['/meshes/example/element/dom_node']['dom_ordinal'] = '3'
            else:
                corrupted['/meshes/example/vertices/0'][0]['vec3'][0]['f64'] = '4000000000000000'
            self.assertEqual(compare(corrupted, rust)['status'], 'differs')

    def test_shape_index_proof_allows_integer_storage_only(self):
        node = '/dom/children/0/children/0'
        oracle = {
            node + '/ordinal': {'u64': '2'},
            node + '/name': {'bytes_hex': b'Shape'.hex()},
            node + '/children/0/name': {'bytes_hex': b'Indexes'.hex()},
            node + '/children/0/values/0/array/0': [{'i64': '0'}, {'i64': '1'}],
            '/blend_shapes/example/element/dom_node': {'dom_ordinal': '2'},
            '/blend_shapes/example/offset_vertices/0': [{'u64': '0'}, {'u64': '1'}],
        }
        rust = copy.deepcopy(oracle)
        rust[node + '/children/0/values/0/array/0'].reverse()
        self.assertEqual(compare(oracle, rust)['status'], 'equal')
        oracle['/blend_shapes/example/offset_vertices/0'][1] = {'u64': '2'}
        self.assertEqual(compare(oracle, rust)['status'], 'differs')

    def test_unknown_dom_change_is_not_explained(self):
        self.assertEqual(compare({'/dom/children/0/name': {'bytes_hex': '41'}}, {'/dom/children/0/name': {'bytes_hex': '42'}})['status'], 'differs')

    def test_axes_difference_needs_source_properties_and_exact_target(self):
        oracle = {}
        for i, (prop, value) in enumerate([('CoordAxis', 0), ('CoordAxisSign', 1), ('UpAxis', 2), ('UpAxisSign', 1), ('FrontAxis', 1), ('FrontAxisSign', -1)]):
            prefix = f'/globals/settings/props/props/{i}/'
            oracle[prefix + 'name'] = {'bytes_hex': prop.encode().hex()}
            oracle[prefix + 'value_int'] = {'i64': str(value)}
        for axis, value in [('right', 0), ('up', 4), ('front', 3)]:
            oracle['/globals/settings/axes/' + axis] = {'enum': 'ufbx_coordinate_axis', 'i64': str(value)}
        rust = copy.deepcopy(oracle)
        rust['/globals/settings/axes/up']['i64'] = '2'
        rust['/globals/settings/axes/front']['i64'] = '4'
        result = compare(oracle, rust)
        self.assertEqual(result['status'], 'equal')
        self.assertEqual(result['explained_axes_fields'], 2)
        for changed in ['/globals/settings/axes/up', '/globals/settings/axes/right', '/globals/settings/props/props/2/value_int']:
            broken = copy.deepcopy(rust)
            broken[changed]['i64'] = '5'
            self.assertEqual(compare(oracle, broken)['status'], 'differs')
        for missing in ['/globals/settings/props/props/2/value_int', '/globals/settings/axes/up']:
            broken = copy.deepcopy(oracle)
            del broken[missing]
            self.assertEqual(compare(broken, rust)['status'], 'differs')
        broken = copy.deepcopy(rust)
        broken['/globals/settings/axes/up']['enum'] = 'other_enum'
        self.assertEqual(compare(oracle, broken)['status'], 'differs')
        broken = copy.deepcopy(rust)
        broken['/globals/settings/unit_meters'] = {'f64': '0000000000000000'}
        self.assertEqual(compare(oracle, broken)['unexplained_fields'], 1)

    def test_missing_order_and_duplicate_paths_are_refused(self):
        with self.assertRaises(ValueError):
            fields(b'{"kind":"field","path":"/graph/a","value":null}\n')
        row = b'{"kind":"field","path":"/order/1/0","value":null}\n'
        with self.assertRaises(ValueError):
            fields(row + row)

    def test_input_binding_is_required_for_each_producer(self):
        row = b'{"kind":"field","path":"/order/1/0","value":null}\n'
        receipt = b'{"kind":"receipt","input_sha256":"a"}\n'
        self.assertTrue(fields(receipt + row, 'a'))
        for data in [row, receipt + row]:
            with self.assertRaises(ValueError):
                fields(data, 'b')


if __name__ == '__main__':
    unittest.main()
