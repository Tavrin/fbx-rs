#!/usr/bin/env python3
"""Generate the Rust canonical field schema from the pinned ufbx header.

Requires pycparser==2.23 (as reference/generate.py). The walk mirrors
generate.py exactly: same union representation choices, exclusions, dense
handles, named-map length records and sorted lists. The output is a static
table that src/canon.rs walks to emit `fbx-scene-canon/1` fields in the same
order as the C oracle. ufbx.h is MIT/Unlicense; it is read as build data.

usage: generate_rust.py UFBX_H OUTPUT_RS
"""
import re
import subprocess
import sys
from pathlib import Path

import pycparser
from pycparser import c_ast, c_parser

PRELUDE = """
typedef unsigned long size_t; typedef long ptrdiff_t;
typedef signed char int8_t; typedef unsigned char uint8_t;
typedef short int16_t; typedef unsigned short uint16_t;
typedef int int32_t; typedef unsigned int uint32_t;
typedef long int64_t; typedef unsigned long uint64_t;
typedef long intptr_t; typedef unsigned long uintptr_t;
typedef _Bool bool;
"""
SPECIAL = {"ufbx_string": "Str", "ufbx_blob": "Blob", "ufbx_vec2": "V2", "ufbx_vec3": "V3",
           "ufbx_vec4": "V4", "ufbx_quat": "Quat", "ufbx_matrix": "Mat"}
NAMED_MAPS = {"ufbx_material_fbx_maps": "maps", "ufbx_material_pbr_maps": "maps", "ufbx_material_features": "features"}
FLOAT = {"float", "double", "ufbx_real"}
OMIT_METADATA = {
    "filename", "relative_root", "raw_filename", "raw_relative_root",
    "original_file_path", "raw_original_file_path", "result_memory_used",
    "temp_memory_used", "result_allocs", "temp_allocs", "element_buffer_size",
}
SORTERS = {"ufbx_connection_list": "Connection", "ufbx_anim_prop_list": "AnimProp",
           "ufbx_prop_override_list": "PropOverride", "ufbx_transform_override_list": "TransformOverride"}


def typename(t):
    if isinstance(t, c_ast.TypeDecl):
        return typename(t.type)
    if isinstance(t, c_ast.IdentifierType):
        return " ".join(t.names)
    if isinstance(t, (c_ast.Struct, c_ast.Enum)):
        return t.name
    raise ValueError(type(t).__name__)


def main():
    if pycparser.__version__ != "2.23":
        raise SystemExit("requires pycparser==2.23")
    header, output = Path(sys.argv[1]), Path(sys.argv[2])
    source = re.sub(r"^\s*#\s*include[^\n]*", "", header.read_text(), flags=re.M)
    source = subprocess.run(
        ["cc", "-E", "-P", "-x", "c", "-DUFBX_STANDARD_C", "-DUFBX_NO_ASSERT", "-"],
        input=PRELUDE + source, text=True, capture_output=True, check=True,
    ).stdout
    ast = c_parser.CParser().parse(source)
    structs, enums, constants = {}, set(), {}

    def const(v):
        if isinstance(v, c_ast.Constant):
            return int(v.value.rstrip("uUlL"), 0)
        if isinstance(v, c_ast.ID):
            return constants[v.name]
        if isinstance(v, c_ast.UnaryOp) and v.op == "-":
            return -const(v.expr)
        if isinstance(v, c_ast.BinaryOp):
            a, b = const(v.left), const(v.right)
            return {"+": a + b, "-": a - b, "<<": a << b, "|": a | b, "*": a * b}[v.op]
        raise ValueError(v)

    class Collect(c_ast.NodeVisitor):
        def visit_Struct(self, node):
            if node.name and node.decls:
                structs[node.name] = node
            self.generic_visit(node)

        def visit_Enum(self, node):
            if node.name:
                enums.add(node.name)
            if node.values:
                nxt = 0
                for e in node.values.enumerators:
                    if e.value is not None:
                        nxt = const(e.value)
                    constants[e.name] = nxt
                    nxt += 1

    Collect().visit(ast)
    elements = set()
    for name, node in structs.items():
        if any(isinstance(d.type, c_ast.Union) and d.type.decls and d.type.decls[0].name == "element" for d in node.decls):
            elements.add(name)
    elements.add("ufbx_element")
    order, bodies = [], {}

    def union_member(owner, choices):
        names = {choice.name: choice for choice in choices if choice.name}
        if owner in NAMED_MAPS:
            named = [c for c in choices if c.name is None and isinstance(c.type, c_ast.Struct)]
            return named[0]
        for name in ("element", "value_real_arr", "value_vec4"):
            if name in names:
                return names[name]
        arrays = [c for c in choices if isinstance(c.type, c_ast.ArrayDecl)]
        if len(arrays) == 1:
            return arrays[0]
        raise ValueError(f"unreviewed union representation in {owner}")

    def excluded(owner, field):
        if field.startswith("_"):
            return True
        if field in {"element_id", "typed_id"} and owner == "ufbx_element":
            return True
        if owner == "ufbx_element" and field == "scene":
            return True
        if owner == "ufbx_metadata" and field in OMIT_METADATA:
            return True
        return (owner, field) in {("ufbx_cache_deformer", "external_cache"), ("ufbx_cache_file", "external_cache"), ("ufbx_mesh", "subdivision_result")}

    def need(n):
        if n in SPECIAL:
            return
        if n not in structs:
            raise ValueError("unknown reachable struct " + n)
        if n in bodies:
            return
        bodies[n] = None
        order.append(n)
        node = structs[n]
        if len(node.decls) == 2 and [d.name for d in node.decls] == ["data", "count"] and isinstance(node.decls[0].type, c_ast.PtrDecl):
            item = node.decls[0].type.type
            sorter = SORTERS.get(n)
            bodies[n] = f"Body::List {{ item: {kind(item)}, sorter: {('Some(Sorter::' + sorter + ')') if sorter else 'None'} }}"
        else:
            fields = []
            walk(n, node.decls, fields)
            named = "None"
            if n in NAMED_MAPS:
                arr = next(d for d in node.decls if isinstance(d.type, c_ast.Union))
                anon = union_member(n, arr.type.decls)
                named = f"Some({len(anon.type.decls)})"
            bodies[n] = f"Body::Fields {{ named_len: {named}, fields: &[\n" + "".join(f"        F {{ name: \"{a}\", kind: {b} }},\n" for a, b in fields) + "    ] }"

    def walk(owner, decls, fields):
        for d in decls:
            if d.name is None and isinstance(d.type, (c_ast.Union, c_ast.Struct)):
                chosen = [union_member(owner, d.type.decls)] if isinstance(d.type, c_ast.Union) else d.type.decls
                walk(owner, chosen, fields)
                continue
            if excluded(owner, d.name):
                continue
            if d.name == "element_id" or (owner == "ufbx_transform_override" and d.name == "node_id"):
                fields.append((d.name, f"K::Handle({str(d.name == 'node_id').lower()})"))
                continue
            fields.append((d.name, kind(d.type)))

    def kind(t):
        if isinstance(t, c_ast.ArrayDecl):
            n = const(t.dim)
            return f"K::Arr(&{kind(t.type)}, {n})"
        if isinstance(t, c_ast.PtrDecl):
            if isinstance(t.type, c_ast.FuncDecl):
                return "K::Present"
            n = typename(t.type)
            if n in elements:
                return "K::Ref"
            if n == "ufbx_dom_node":
                return "K::DomRef"
            if n in {"void", "char", "ufbx_geometry_cache"} or n.endswith("_fn"):
                return "K::Present"
            need(n)
            return f"K::Opt(\"{n}\")"
        n = typename(t)
        if n in SPECIAL:
            return "K::" + SPECIAL[n]
        if n in FLOAT:
            return "K::F32" if n == "float" else "K::F64"
        if n in {"bool", "_Bool"}:
            return "K::Bool"
        if n in enums:
            return f"K::Enum(\"{n}\")"
        if n in structs:
            need(n)
            return f"K::S(\"{n}\")"
        unsigned = n.startswith("u") or n in {"size_t", "unsigned int", "unsigned long", "unsigned char", "unsigned short"}
        return "K::U64" if unsigned else "K::I64"

    for n in sorted(elements | {"ufbx_metadata", "ufbx_scene_settings", "ufbx_load_opts", "ufbx_texture_file_list", "ufbx_anim", "ufbx_transform"}):
        need(n)
    out = ["// @generated by reference/generate_rust.py from the pinned ufbx.h (C 0.23.1).",
           "// Do not edit by hand; regenerate and review the diff.",
           "use crate::canon::{Body, Sorter, F, K};", "",
           "pub(crate) static STRUCTS: &[(&str, Body)] = &["]
    for n in sorted(order):
        out.append(f"    (\"{n}\", {bodies[n]}),")
    out.append("];")
    out.append("")
    out.append("/// Element struct names (the `element` union first member), for review.")
    out.append("#[allow(dead_code)]")
    out.append("pub(crate) static ELEMENT_STRUCTS: &[&str] = &[" + ", ".join(f'"{n}"' for n in sorted(elements - {"ufbx_element"})) + "];")
    output.write_text("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
