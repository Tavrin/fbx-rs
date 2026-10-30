/* Exact-bit animation bake oracle. Compile against hash-pinned ufbx 0.23.1
 * with -ffp-contract=off. No external file requests are allowed. */
#include "ufbx.h"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <inttypes.h>
static uint64_t bits(double v) { uint64_t b; memcpy(&b,&v,8); return b; }
static bool deny(void *u,ufbx_stream *s,const char *n,size_t l,const ufbx_open_file_info *i) {(void)u;(void)s;(void)n;(void)l;(void)i;return false;}
static int64_t id(ufbx_element *e) { return e->dom_node && e->dom_node->values.count ? e->dom_node->values.data[0].value_int : 0; }
static void hex(ufbx_string s) { if(!s.length) putchar('-');for(size_t i=0;i<s.length;i++)printf("%02x",(unsigned char)s.data[i]); }
static void vecs(const char *kind,int64_t stack,ufbx_element *e,const char *ch,ufbx_baked_vec3_list list,ufbx_string prop) {
 for(size_t i=0;i<list.count;i++) {ufbx_baked_vec3 k=list.data[i];printf("%s %" PRId64 " %" PRId64 ":",kind,stack,id(e));hex(prop);if(ch)printf(" %s",ch);printf(" %zu %016" PRIx64 " %u %016" PRIx64 " %016" PRIx64 " %016" PRIx64 "\n",i,bits(k.time),(unsigned)k.flags,bits(k.value.x),bits(k.value.y),bits(k.value.z));}
}
int main(int argc,char **argv) {
 if(argc<2)return 2;
 FILE *f=fopen(argv[1],"rb");if(!f)return 2;fseek(f,0,SEEK_END);long len=ftell(f);rewind(f);if(len<0){fclose(f);return 2;}void *data=malloc((size_t)len);if(!data){fclose(f);return 2;}if(fread(data,1,(size_t)len,f)!=(size_t)len)return 2;fclose(f);
 ufbx_load_opts o={0};o.file_format=UFBX_FILE_FORMAT_FBX;o.retain_dom=true;o.open_file_cb.fn=deny;o.load_external_files=false;
 o.target_axes=ufbx_axes_right_handed_y_up;o.target_unit_meters=1.0;o.target_camera_axes=ufbx_axes_right_handed_y_up;o.target_light_axes=ufbx_axes_right_handed_y_up;
 o.space_conversion=UFBX_SPACE_CONVERSION_MODIFY_GEOMETRY;o.geometry_transform_handling=UFBX_GEOMETRY_TRANSFORM_HANDLING_MODIFY_GEOMETRY;o.inherit_mode_handling=UFBX_INHERIT_MODE_HANDLING_COMPENSATE;o.handedness_conversion_axis=UFBX_MIRROR_AXIS_Z;o.generate_missing_normals=true;o.normalize_normals=true;
 ufbx_error err;ufbx_scene *s=ufbx_load_memory(data,(size_t)len,&o,&err);free(data);if(!s)return 3;
 ufbx_bake_opts opts={0};opts.trim_start_time=argc>2 && strstr(argv[2],"trim");opts.key_reduction_enabled=!(argc>2 && strstr(argv[2],"raw"));opts.key_reduction_rotation=argc>2 && strstr(argv[2],"rotation");
 opts.temp_allocator.memory_limit=(size_t)768<<20;opts.result_allocator.memory_limit=(size_t)768<<20;
 for(size_t ix=0;ix<s->anim_stacks.count;ix++) {
  ufbx_anim_stack *st=s->anim_stacks.data[ix];int64_t si=id(&st->element);ufbx_baked_anim *b=ufbx_bake_anim(s,st->anim,&opts,&err);if(!b){ufbx_free_scene(s);return 4;}
  printf("S %" PRId64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 "\n",si,bits(b->playback_time_begin),bits(b->playback_time_end),bits(b->playback_duration),bits(b->key_time_min),bits(b->key_time_max));
  for(size_t i=0;i<b->nodes.count;i++) {
   ufbx_baked_node *n=&b->nodes.data[i];ufbx_node *node=s->nodes.data[n->typed_id];ufbx_element *e=&node->element;
   printf("N %" PRId64 " %" PRId64 ":",si,id(e));hex(e->name);printf(" %u %u %u %zu %zu %zu\n",n->constant_translation,n->constant_rotation,n->constant_scale,n->translation_keys.count,n->rotation_keys.count,n->scale_keys.count);
   vecs("K",si,e,"t",n->translation_keys,e->name);vecs("K",si,e,"s",n->scale_keys,e->name);
   for(size_t j=0;j<n->rotation_keys.count;j++) {ufbx_baked_quat k=n->rotation_keys.data[j];printf("K %" PRId64 " %" PRId64 ":",si,id(e));hex(e->name);printf(" r %zu %016" PRIx64 " %u %016" PRIx64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 "\n",j,bits(k.time),(unsigned)k.flags,bits(k.value.x),bits(k.value.y),bits(k.value.z),bits(k.value.w));}
  }
  for(size_t i=0;i<b->elements.count;i++) {ufbx_baked_element *be=&b->elements.data[i];ufbx_element *e=s->elements.data[be->element_id];for(size_t j=0;j<be->props.count;j++) {ufbx_baked_prop *p=&be->props.data[j];printf("P %" PRId64 " %" PRId64 ":",si,id(e));hex(p->name);printf(" %u %zu\n",p->constant_value,p->keys.count);vecs("Q",si,e,NULL,p->keys,p->name);}}
  ufbx_free_baked_anim(b);
 }
 ufbx_free_scene(s);return 0;
}
