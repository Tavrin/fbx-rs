#!/usr/bin/env python3
"""Self-authored integrity, hierarchy and raw-string-array regressions."""
import hashlib
import io
import json
import unittest
from validate_dom import validate


def line(value):
    return (json.dumps(value, separators=(',', ':')) + '\n').encode()


def stream(rows):
    body = b''.join(line(row) for row in rows)
    return body + line({'kind':'footer','records':len(rows)-1,'sha256':hashlib.sha256(body).hexdigest(),'oracle_exit':0})


def header():
    h = {'kind':'header','schema':'fbx-ufbx-dom/2','input_length':0,'retain_dom':True,'external_files':False,
         'open_main_file_with_default':True,'file_format':'FBX','temp_allocator_bytes':805306368,'result_allocator_bytes':805306368}
    for key in ('input_sha256','source_sha256','header_sha256','dumper_sha256','runner_sha256','build_receipt_sha256','executable_sha256'):
        h[key] = '00' * 32
    return h


def node(depth, values, children):
    return {'kind':'node','depth':depth,'name_hex':'','values':values,'children':children}


def value(blob):
    return {'kind':'value','type':7,'integer':'1','float64_hex':'3ff0000000000000','string_hex':'','blob_hex':blob}


class DomValidatorTest(unittest.TestCase):
    def test_valid_raw_string_array(self):
        data = stream([header(), node(0, 0, 1), node(1, 1, 0), value('0000000000000003' + '610062')])
        self.assertEqual(validate(io.BytesIO(data))['records'], 3)

    def test_rejects_corruption_truncation_and_trailing(self):
        data = stream([header(), node(0, 0, 0)])
        for changed in (data.replace(b'"children":0', b'"children":1'), data[:-1], data + b'x'):
            with self.assertRaises(ValueError): validate(io.BytesIO(changed))

    def test_rejects_bad_tree_and_raw_string_length_even_with_valid_footer(self):
        cases = [[header(), node(0, 0, 1), node(2, 0, 0)],
                 [header(), node(0, 1, 0), value('0000000000000004' + '610062')]]
        for rows in cases:
            with self.assertRaises(ValueError): validate(io.BytesIO(stream(rows)))

    def test_rejects_duplicate_key(self):
        with self.assertRaises(ValueError): validate(io.BytesIO(b'{"kind":"header","kind":"header"}\n'))


if __name__ == '__main__': unittest.main()
