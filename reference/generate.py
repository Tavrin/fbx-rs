#!/usr/bin/env python3
"""Generate exhaustive field walkers from a pinned header, never from asset data.

Requires pycparser==2.23. Unrecognized reachable types are fatal. The coverage
manifest includes union aliases and explicitly excluded implementation fields.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path
import pycparser
from pycparser import c_ast, c_parser, c_generator

PRELUDE = """
typedef unsigned long size_t; typedef long ptrdiff_t;
typedef signed char int8_t; typedef unsigned char uint8_t;
typedef short int16_t; typedef unsigned short uint16_t;
typedef int int32_t; typedef unsigned int uint32_t;
typedef long int64_t; typedef unsigned long uint64_t;
typedef long intptr_t; typedef unsigned long uintptr_t;
typedef _Bool bool;
"""
SPECIAL = {"ufbx_string", "ufbx_blob", "ufbx_vec2", "ufbx_vec3", "ufbx_vec4", "ufbx_quat", "ufbx_matrix"}
NAMED_MAPS = {"ufbx_material_fbx_maps": "maps", "ufbx_material_pbr_maps": "maps", "ufbx_material_features": "features"}
FLOAT = {"float", "double", "ufbx_real"}
OMIT_METADATA = {
    "filename", "relative_root", "raw_filename", "raw_relative_root",
    "original_file_path", "raw_original_file_path", "result_memory_used",
    "temp_memory_used", "result_allocs", "temp_allocs", "element_buffer_size",
}


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
        raise SystemExit("the field generator requires pycparser==2.23")
    ap = argparse.ArgumentParser()
    ap.add_argument("header", type=Path)
    ap.add_argument("output", type=Path)
    ap.add_argument("coverage", type=Path)
    args = ap.parse_args()
    source = re.sub(r"^\s*#\s*include[^\n]*", "", args.header.read_text(), flags=re.M)
    source = subprocess.run(
        ["cc", "-E", "-P", "-x", "c", "-DUFBX_STANDARD_C", "-DUFBX_NO_ASSERT", "-"],
        input=PRELUDE + source, text=True, capture_output=True, check=True,
    ).stdout
    ast = c_parser.CParser().parse(source)
    structs, enums = {}, set()

    class Collect(c_ast.NodeVisitor):
        def visit_Struct(self, node):
            if node.name and node.decls:
                structs[node.name] = node
            self.generic_visit(node)

        def visit_Enum(self, node):
            if node.name:
                enums.add(node.name)

    Collect().visit(ast)
    elements = set()
    for name, node in structs.items():
        if any(isinstance(d.type, c_ast.Union) and d.type.decls and d.type.decls[0].name == "element" for d in node.decls):
            elements.add(name)
    elements.add("ufbx_element")
    reachable, bodies, coverage, assertions = set(), {}, [], []

    def union_member(owner, choices):
        # Choose a complete representation explicitly. Declaration order is not
        # storage width: material maps and shader inputs put their scalar first.
        names = {choice.name: choice for choice in choices if choice.name}
        if owner in NAMED_MAPS:
            named = [choice for choice in choices if choice.name is None and isinstance(choice.type, c_ast.Struct)]
            if len(named) != 1:
                raise ValueError("named map layout changed: " + owner)
            return named[0]
        for name in ("element", "value_real_arr", "value_vec4"):
            if name in names:
                return names[name]
        arrays = [choice for choice in choices if isinstance(choice.type, c_ast.ArrayDecl)]
        if len(arrays) == 1:
            return arrays[0]
        raise ValueError(f"unreviewed union representation in {owner}: {list(names)}")

    def coverage_alias(owner, declaration, representation):
        if declaration.name is None and isinstance(declaration.type, (c_ast.Struct, c_ast.Union)):
            for child in declaration.type.decls:
                coverage_alias(owner, child, representation)
        else:
            impl = declaration.name in {"element_id", "typed_id"} or declaration.name.startswith("_")
            coverage.append({"struct": owner, "field": declaration.name, "class": "receipt-only" if impl else "exact", "reason": "union alias represented by complete storage member; compile-time size checked", "representation": representation})

    def exclusion(owner, field):
        if field.startswith("_"):
            return "receipt-only", "private implementation/cache field; never an identity"
        if field in {"element_id", "typed_id"} and owner == "ufbx_element":
            return "receipt-only", "dense handles replaced by source identity"
        if owner == "ufbx_element" and field == "scene":
            return "receipt-only", "back pointer; containing scene is implicit"
        if owner == "ufbx_metadata" and field in OMIT_METADATA:
            return "receipt-only", "source filename or allocator bookkeeping excluded from equality"
        if (owner, field) in {("ufbx_cache_deformer", "external_cache"), ("ufbx_cache_file", "external_cache"), ("ufbx_mesh", "subdivision_result")}:
            return "unsupported", "external cache/subdivision evaluation is beyond the six-domain oracle contract"
        return None

    def primitive(t):
        if isinstance(t, c_ast.PtrDecl):
            return typename(t.type) in elements or typename(t.type) == "ufbx_dom_node"
        return not isinstance(t, c_ast.ArrayDecl) and (typename(t) in SPECIAL or typename(t) not in structs)

    def scalar(t, expr):
        if isinstance(t, c_ast.PtrDecl):
            n = typename(t.type)
            if n in elements:
                return f"value_ref((const ufbx_element*)({expr}));"
            if n == "ufbx_dom_node":
                return f"value_dom_ref({expr});"
            raise ValueError(n)
        n = typename(t)
        if n in SPECIAL:
            return f"value_{n}({expr});"
        if n in FLOAT:
            return f"value_{'f32' if n == 'float' else 'f64'}({expr});"
        if n in {"bool", "_Bool"}:
            return f"value_bool({expr});"
        if n in enums:
            return f'value_enum("{n}", (int64_t)({expr}));'
        if n in structs:
            raise ValueError(n)
        return f"value_{'u64' if n.startswith('u') or n in {'size_t', 'unsigned int', 'unsigned long', 'unsigned char', 'unsigned short'} else 'i64'}({expr});"

    def emit(t, expr, depth=0):
        if isinstance(t, c_ast.ArrayDecl):
            item = t.type
            count = f"(sizeof({expr})/sizeof(({expr})[0]))"
            return array(item, expr, count, depth)
        if isinstance(t, c_ast.PtrDecl):
            if isinstance(t.type, c_ast.FuncDecl):
                return f"leaf_begin(); value_bool(({expr}) != NULL); leaf_end();"
            n = typename(t.type)
            if n in elements or n == "ufbx_dom_node":
                return "leaf_begin(); " + scalar(t, expr) + " leaf_end();"
            if n in {"void", "char", "ufbx_geometry_cache"} or n.endswith("_fn"):
                return f"leaf_begin(); value_bool(({expr}) != NULL); leaf_end();"
            need(n)
            return f"if ({expr}) {{ descend(); dump_{n}({expr}); ascend(); }} else {{ leaf_null(); }}"
        n = typename(t)
        if n in SPECIAL or n not in structs:
            return "leaf_begin(); " + scalar(t, expr) + " leaf_end();"
        need(n)
        return f"dump_{n}(&({expr}));"

    def array(item, expr, count, depth):
        i, b = f"i{depth}", f"b{depth}"
        code = f'field_count("length", {count});\n'
        if primitive(item):
            code += f'for (size_t {b}=0; {b}<{count}; {b}+=4096) {{ size_t mark=path_index({b}); leaf_begin(); out("[");\n'
            code += f'for (size_t {i}={b}; {i}<{count} && {i}-{b}<4096; {i}++) {{ if ({i}!={b}) out(","); {scalar(item, f"({expr})[{i}]")} }}\nout("]"); leaf_end(); path_restore(mark); }}'
        else:
            code += f'for (size_t {i}=0; {i}<{count}; {i}++) {{ size_t mark=path_index({i}); {emit(item, f"({expr})[{i}]", depth+1)} path_restore(mark); }}'
        return code

    def fields(owner, decls, prefix):
        code = []
        for d in decls:
            if d.name is None and isinstance(d.type, (c_ast.Union, c_ast.Struct)):
                choices = d.type.decls
                if isinstance(d.type, c_ast.Union):
                    selected = union_member(owner, choices)
                    chosen = [selected]
                    representation = selected.name or "(named fields in anonymous struct)"
                    selected_size = f"sizeof((({owner}*)0)->{selected.name})" if selected.name else "sizeof(" + c_generator.CGenerator().visit(selected.type) + ")"
                    for alias in choices:
                        if alias is selected:
                            continue
                        coverage_alias(owner, alias, representation)
                        alias_size = f"sizeof((({owner}*)0)->{alias.name})" if alias.name else "sizeof(" + c_generator.CGenerator().visit(alias.type) + ")"
                        assertions.append(f'_Static_assert({selected_size} >= {alias_size}, "{owner} union storage coverage");')
                else:
                    chosen = choices
                code.extend(fields(owner, chosen, prefix))
                continue
            if not d.name:
                raise ValueError((owner, d))
            ex = exclusion(owner, d.name)
            handle = d.name == "element_id" or (owner == "ufbx_transform_override" and d.name == "node_id")
            coverage.append({"struct": owner, "field": d.name, "class": ex[0] if ex else "exact", "reason": ex[1] if ex else "dense reference replaced by source identity" if handle else "all loaded values exact; evaluation emitted separately"})
            if ex:
                continue
            if handle:
                code.append(f'{{ size_t mark=path_field("{d.name}"); leaf_begin(); value_handle({prefix + d.name}, {str(d.name == "node_id").lower()}); leaf_end(); path_restore(mark); }}')
                continue
            code.append(f'{{ size_t mark=path_field("{d.name}"); {emit(d.type, prefix + d.name)} path_restore(mark); }}')
        return code

    def need(n):
        if n in reachable or n in SPECIAL:
            return
        if n not in structs:
            raise ValueError("unknown reachable struct " + str(n))
        reachable.add(n)
        node = structs[n]
        if len(node.decls) == 2 and [d.name for d in node.decls] == ["data", "count"] and isinstance(node.decls[0].type, c_ast.PtrDecl):
            item = node.decls[0].type.type
            coverage.extend({"struct": n, "field": d.name, "class": "exact", "reason": "ordered sequence, explicit length, 4096-element chunks"} for d in node.decls)
            if not isinstance(item, c_ast.PtrDecl) and typename(item) == "void":
                raise ValueError("untyped list needs explicit representation: " + n)
            bodies[n] = array(item, "v->data", "v->count", 0)
            sorters = {"ufbx_connection_list": "connection_order", "ufbx_anim_prop_list": "anim_prop_order", "ufbx_prop_override_list": "prop_override_order", "ufbx_transform_override_list": "transform_override_order"}
            if n in sorters:
                body = bodies[n]
                bodies[n] = f"{n} sorted=*v; sorted.data=stable_sorted_copy(v->data,v->count,sizeof(*v->data),{sorters[n]});\n{{ const {n} *v=&sorted; {body} }}\nfree(sorted.data);"
        else:
            bodies[n] = "\n".join(fields(n, node.decls, "v->"))
            if n in NAMED_MAPS:
                array_name = NAMED_MAPS[n]
                bodies[n] = f'field_count("length", sizeof(v->{array_name})/sizeof(v->{array_name}[0]));\n' + bodies[n]

    for n in sorted(elements | {"ufbx_metadata", "ufbx_scene_settings", "ufbx_load_opts", "ufbx_texture_file_list", "ufbx_anim", "ufbx_transform"}):
        need(n)
    def manual_fields(owner, declarations, reason):
        for d in declarations:
            if d.name is None and isinstance(d.type, (c_ast.Struct, c_ast.Union)):
                manual_fields(owner, d.type.decls, reason)
            else:
                coverage.append({"struct": owner, "field": d.name, "class": "exact", "reason": reason})

    for n in ("ufbx_scene", "ufbx_dom_node", "ufbx_dom_value"):
        manual_fields(n, structs[n].decls, "manual serializer: scene/domain sections and stable indices; DOM arrays decode values, never pointer backing storage")
    args.output.write_text("// Generated from the verified header by reference/generate.py.\n" +
        "\n".join(assertions) + "\n" +
        "\n".join(f"static void dump_{n}(const {n} *v);" for n in sorted(reachable)) + "\n" +
        "\n".join(f"static void dump_{n}(const {n} *v) {{\n{bodies[n]}\n}}" for n in sorted(reachable)) + "\n" +
        "static void dump_element(const ufbx_element *e) { switch(e->type) {\n" +
        "\n".join(f"case UFBX_ELEMENT_{n[5:].upper()}: dump_{n}((const {n}*)e); break;" for n in sorted(elements - {"ufbx_element"})) +
        '\ndefault: fail("unrecognized element type"); } }\n')
    # Special leaf storage has no hidden values: strings/blobs include their
    # lengths; vectors include every component of the first union member.
    for n in sorted(SPECIAL):
        manual_fields(n, structs[n].decls, "special leaf: all bytes/components and explicit type; blob bytes chunked")
    args.coverage.write_text(json.dumps({"schema": "fbx-field-coverage/1", "reachable_structs": sorted(reachable | SPECIAL), "fields": coverage, "advanced_evaluation": "unsupported: constraints, caches, NURBS, LOD, selection/display; typed fields and DOM preserved"}, indent=2) + "\n")


if __name__ == "__main__":
    main()
