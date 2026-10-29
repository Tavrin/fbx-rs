/* Retained DOM diagnostic for the pinned historical ufbx profile.
 * Output contains the values ufbx retains, not necessarily original FBX type codes.
 * ufbx is MIT OR Unlicense: https://github.com/ufbx/ufbx/blob/master/LICENSE
 */
#include "ufbx.h"
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void hex(const void *ptr, size_t size) {
    const unsigned char *p = (const unsigned char*)ptr;
    for (size_t i = 0; i < size; i++) printf("%02x", p[i]);
}
static void value(const ufbx_dom_value *v) {
    uint64_t bits;
    memcpy(&bits, &v->value_float, sizeof(bits));
    printf("V %u %" PRId64 " %016" PRIx64 " ", (unsigned)v->type, v->value_int, bits);
    hex(v->value_str.data, v->value_str.length);
    printf(" ");
    size_t width = 1;
    if (v->type == UFBX_DOM_VALUE_ARRAY_I32 || v->type == UFBX_DOM_VALUE_ARRAY_F32) width = 4;
    if (v->type == UFBX_DOM_VALUE_ARRAY_I64 || v->type == UFBX_DOM_VALUE_ARRAY_F64) width = 8;
    const unsigned char *p = (const unsigned char*)v->value_blob.data;
    if (v->type == UFBX_DOM_VALUE_ARRAY_RAW_STRING) {
        if (v->value_blob.size % sizeof(ufbx_string) != 0) { fprintf(stderr, "bad string array size\n"); exit(2); }
        for (size_t i = 0; i < v->value_blob.size; i += sizeof(ufbx_string)) {
            ufbx_string str; memcpy(&str, p + i, sizeof(str));
            printf("%016" PRIx64, (uint64_t)str.length); hex(str.data, str.length);
        }
        printf("\n"); return;
    }
    if (v->value_blob.size % width != 0) { fprintf(stderr, "bad array size\n"); exit(2); }
    for (size_t i = 0; i < v->value_blob.size; i += width) {
        if (width == 1) printf("%02x", p[i]);
        else if (width == 4) { uint32_t x; memcpy(&x, p + i, 4); printf("%08" PRIx32, x); }
        else { uint64_t x; memcpy(&x, p + i, 8); printf("%016" PRIx64, x); }
    }
    printf("\n");
}
static bool deny_open(void *user, ufbx_stream *stream, const char *name, size_t len, const ufbx_open_file_info *info) {
    (void)user; (void)stream; (void)name; (void)len; (void)info; return false;
}
typedef struct { const ufbx_dom_node *node; size_t next; } frame;
int main(int argc, char **argv) {
    if (argc != 2) { fprintf(stderr, "usage: dom FILE\n"); return 2; }
    ufbx_load_opts opts = { 0 }; opts.retain_dom = true; opts.file_format = UFBX_FILE_FORMAT_FBX; opts.open_file_cb.fn = deny_open; opts.open_main_file_with_default = true; opts.load_external_files = false;
    opts.temp_allocator.memory_limit = 768u * 1024u * 1024u;
    opts.result_allocator.memory_limit = 768u * 1024u * 1024u;
    ufbx_error error = {0};
    ufbx_scene *scene = ufbx_load_file(argv[1], &opts, &error);
    if (!scene || !scene->dom_root) {
        const char *message = error.description.data ? error.description.data : "no DOM";
        size_t length = error.description.data ? error.description.length : 6;
        if (length > 1000) length = 1000;
        fprintf(stderr, "ufbx load failed: %.*s\n", (int)length, message);
        if (scene) ufbx_free_scene(scene);
        return 1;
    }
    frame *stack = NULL; size_t depth = 0, capacity = 0;
    const ufbx_dom_node *node = scene->dom_root;
    puts("fbx-ufbx-dom/2");
    for (;;) {
        printf("N %zu ", depth); hex(node->name.data, node->name.length); printf(" %zu %zu\n", node->values.count, node->children.count);
        for (size_t i = 0; i < node->values.count; i++) value(&node->values.data[i]);
        if (node->children.count) {
            if (depth == capacity) {
                if (capacity > SIZE_MAX / 2 / sizeof(*stack)) { free(stack); ufbx_free_scene(scene); return 2; }
                size_t n = capacity ? capacity * 2 : 16;
                frame *p = (frame*)realloc(stack, n * sizeof(*stack));
                if (!p) { free(stack); ufbx_free_scene(scene); return 2; }
                stack = p; capacity = n;
            }
            stack[depth++] = (frame){node, 1}; node = node->children.data[0]; continue;
        }
        while (depth && stack[depth-1].next == stack[depth-1].node->children.count) depth--;
        if (!depth) break;
        frame *f = &stack[depth-1]; node = f->node->children.data[f->next++];
    }
    free(stack); ufbx_free_scene(scene);
    return ferror(stdout) ? 2 : 0;
}
