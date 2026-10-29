// Inventory/timing probe, NOT the canonical parity dumper.
// Build against either upstream C v0.11.0 or the C bundled by Rust ufbx 0.11.5.
#define _POSIX_C_SOURCE 200809L
#include "ufbx.h"
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/resource.h>

static uint64_t cpu_ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t)) abort();
    return (uint64_t)t.tv_sec * UINT64_C(1000000000) + (uint64_t)t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 4) { fprintf(stderr,"usage: scene_probe FILE REPEATS raw|profile-options\n"); return 2; }
    char *end = NULL;
    unsigned long repeats = strtoul(argv[2], &end, 10);
    if (!end || *end || repeats == 0 || repeats > 100000) return 2;
    bool profile = strcmp(argv[3], "profile-options") == 0;
    if (!profile && strcmp(argv[3], "raw") != 0) return 2;
    FILE *f = fopen(argv[1], "rb");
    if (!f || fseek(f,0,SEEK_END)) return 3;
    long length = ftell(f);
    if (length < 0 || (uint64_t)length > (UINT64_C(2)<<30) || fseek(f,0,SEEK_SET)) return 3;
    void *data = malloc(length ? (size_t)length : 1);
    if (!data || fread(data,1,(size_t)length,f)!=(size_t)length) return 3;
    fclose(f);
    ufbx_load_opts opts = {0};
    // CPU-only bounded reference experiment: 768 MiB per allocator, <=1.5 GiB total.
    opts.temp_allocator.memory_limit = (size_t)768 << 20;
    opts.result_allocator.memory_limit = (size_t)768 << 20;
    opts.temp_allocator.allocation_limit = 1 << 24;
    opts.result_allocator.allocation_limit = 1 << 24;
    opts.file_format = UFBX_FILE_FORMAT_FBX;
    opts.load_external_files = false;
    if (profile) {
        opts.target_axes = ufbx_axes_right_handed_y_up;
        opts.target_unit_meters = 1.0;
        opts.target_camera_axes = ufbx_axes_right_handed_y_up;
        opts.target_light_axes = ufbx_axes_right_handed_y_up;
        opts.space_conversion = UFBX_SPACE_CONVERSION_MODIFY_GEOMETRY;
        opts.geometry_transform_handling = UFBX_GEOMETRY_TRANSFORM_HANDLING_MODIFY_GEOMETRY;
        opts.inherit_mode_handling = UFBX_INHERIT_MODE_HANDLING_COMPENSATE;
        opts.handedness_conversion_axis = UFBX_MIRROR_AXIS_Z;
        opts.generate_missing_normals = true;
        opts.normalize_normals = true;
    }
    uint64_t load=0, release=0;
    for (unsigned long r=0;r<repeats;r++) {
        ufbx_error error = {0};
        uint64_t t=cpu_ns();
        ufbx_scene *s=ufbx_load_memory(data,(size_t)length,&opts,&error);
        load+=cpu_ns()-t;
        if (!s) { fprintf(stderr,"ufbx error %d: %.*s\n",(int)error.type,(int)error.description.length,error.description.data); free(data); return 4; }
        if (r+1==repeats) {
            size_t vertices=0,faces=0,uv_max=0,embedded=0,keys=0;
            for(size_t i=0;i<s->meshes.count;i++) {
                ufbx_mesh *m=s->meshes.data[i]; vertices+=m->num_vertices; faces+=m->num_faces;
                if(m->uv_sets.count>uv_max) uv_max=m->uv_sets.count;
            }
            for(size_t i=0;i<s->textures.count;i++) if(s->textures.data[i]->content.size) embedded++;
            for(size_t i=0;i<s->anim_curves.count;i++) keys+=s->anim_curves.data[i]->keyframes.count;
            printf("{\"probe_schema\":1,\"ufbx_source_version\":%u,\"profile\":\"%s\",\"repeats\":%lu,\"load_cpu_ns_total\":%" PRIu64 ",",ufbx_source_version,argv[3],repeats,load);
            printf("\"ascii\":%s,\"version\":%u,\"nodes\":%zu,\"meshes\":%zu,\"vertices\":%zu,\"faces\":%zu,",s->metadata.ascii?"true":"false",s->metadata.version,s->nodes.count,s->meshes.count,vertices,faces);
            printf("\"skin_deformers\":%zu,\"blend_deformers\":%zu,\"blend_shapes\":%zu,\"anim_stacks\":%zu,\"anim_curves\":%zu,\"keyframes\":%zu,\"uv_sets_max\":%zu,",s->skin_deformers.count,s->blend_deformers.count,s->blend_shapes.count,s->anim_stacks.count,s->anim_curves.count,keys,uv_max);
            printf("\"materials\":%zu,\"textures\":%zu,\"embedded_textures\":%zu,\"cameras\":%zu,\"lights\":%zu,\"unit_meters\":%.17g,\"axes\":[%d,%d,%d],",s->materials.count,s->textures.count,embedded,s->cameras.count,s->lights.count,(double)s->settings.unit_meters,(int)s->settings.axes.right,(int)s->settings.axes.up,(int)s->settings.axes.front);
            printf("\"result_memory_bytes\":%zu,\"temp_memory_bytes\":%zu,\"warnings\":%zu,",s->metadata.result_memory_used,s->metadata.temp_memory_used,s->metadata.warnings.count);
        }
        t=cpu_ns(); ufbx_free_scene(s); release+=cpu_ns()-t;
    }
    struct rusage ru; if(getrusage(RUSAGE_SELF,&ru)) return 5;
    printf("\"free_cpu_ns_total\":%" PRIu64 ",\"max_rss_kib\":%ld}\n",release,ru.ru_maxrss);
    free(data);
    return 0;
}
