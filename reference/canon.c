// Canonical ufbx oracle. ufbx source is an independently hash-pinned build input.
#define _POSIX_C_SOURCE 200809L
#include "ufbx.h"
#include "config.h"
#include <openssl/evp.h>
#include <inttypes.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <limits.h>
#include <float.h>
_Static_assert(sizeof(ufbx_real)==8 && sizeof(double)==8 && sizeof(float)==4 && DBL_MANT_DIG==53 && FLT_MANT_DIG==24,"IEEE f64/f32 required");

static EVP_MD_CTX *file_hash, *section_hash;
static bool hash_file = true, hash_section;
static uint64_t section_count, total_records, work;
static char path[16384];
static size_t path_len, recursion_depth;
static const ufbx_scene *scene;
typedef struct { const ufbx_dom_node *node; size_t ordinal; } dom_entry;
static dom_entry *dom_entries;
static size_t num_dom, cap_dom;
typedef struct { const ufbx_element *element; int64_t source_id; size_t ordinal; char *key; } element_entry;
static element_entry *element_entries, **ordered_elements;
static size_t num_elements;

static void fail(const char *message) { fprintf(stderr, "canonical oracle: %s\n", message); exit(5); }
static void *allocate(size_t n) { void *p=malloc(n ? n : 1); if (!p) fail("allocation failed"); return p; }
static void charge(void) { if (++work > UINT64_C(50000000)) fail("serialization/evaluation work limit"); }
static void out_n(const char *s, size_t n) {
    if (fwrite(s,1,n,stdout)!=n) fail("output write failed");
    if (hash_file && !EVP_DigestUpdate(file_hash,s,n)) fail("SHA-256 update");
    if (hash_section && !EVP_DigestUpdate(section_hash,s,n)) fail("section SHA-256 update");
}
static void out(const char *s) { out_n(s,strlen(s)); }
static void outf(const char *format, ...) {
    char buffer[512]; va_list args; va_start(args,format);
    int n=vsnprintf(buffer,sizeof(buffer),format,args); va_end(args);
    if (n<0 || (size_t)n>=sizeof(buffer)) fail("format buffer limit");
    out(buffer);
}
static void json_string(const char *s) {
    out("\"");
    const unsigned char *p=(const unsigned char*)s;
    while(*p) {
        const unsigned char *begin=p;
        while(*p && *p!='"' && *p!='\\' && *p>=32 && *p<127)p++;
        if(p!=begin)out_n((const char*)begin,(size_t)(p-begin));
        if(!*p)break;
        if (*p=='"' || *p=='\\') outf("\\%c",*p);
        else if (*p<32 || *p>=127) outf("\\u%04x",*p);
        p++;
    }
    out("\"");
}
static void digest_hex(const void *data, size_t n, char hex[65]) {
    unsigned char digest[32]; unsigned int length;
    if (!EVP_Digest(data,n,digest,&length,EVP_sha256(),NULL) || length!=32) fail("SHA-256");
    for (size_t i=0;i<32;i++) sprintf(hex+2*i,"%02x",digest[i]);
    hex[64]=0;
}
static void finish_hash(EVP_MD_CTX *ctx, char hex[65]) {
    unsigned char digest[32]; unsigned int n;
    if (!EVP_DigestFinal_ex(ctx,digest,&n) || n!=32) fail("SHA-256 final");
    for (size_t i=0;i<32;i++) sprintf(hex+2*i,"%02x",digest[i]);
    hex[64]=0;
}
static void hex_bytes(const void *data, size_t n) {
    const unsigned char *p=data; out("\"");
    char buffer[8193];
    while (n) { size_t c=n>4096?4096:n; for(size_t i=0;i<c;i++) sprintf(buffer+i*2,"%02x",p[i]); buffer[2*c]=0; out(buffer); p+=c; n-=c; }
    out("\"");
}
static size_t path_field(const char *s) {
    size_t mark=path_len, n=strlen(s);
    if (path_len+n+2>=sizeof(path)) fail("field path limit");
    path[path_len++]='/'; memcpy(path+path_len,s,n+1); path_len+=n; return mark;
}
static size_t path_index(size_t i) { char s[32]; snprintf(s,sizeof(s),"%zu",i); return path_field(s); }
static void path_restore(size_t mark) { path_len=mark; path[mark]=0; }
static void descend(void) { if (++recursion_depth>256) fail("default/pointer depth limit or cycle"); }
static void ascend(void) { --recursion_depth; }
static void leaf_begin(void) { charge(); out("{\"kind\":\"field\",\"path\":"); json_string(path); out(",\"value\":"); }
static void leaf_end(void) { out("}\n"); section_count++; total_records++; }
static void leaf_null(void) { leaf_begin(); out("null"); leaf_end(); }
static void value_bool(bool v) { out(v?"true":"false"); }
static void value_i64(int64_t v) { outf("{\"i64\":\"%" PRId64 "\"}",v); }
static void value_u64(uint64_t v) { outf("{\"u64\":\"%" PRIu64 "\"}",v); }
static void value_enum(const char *type, int64_t v) { outf("{\"enum\":\"%s\",\"i64\":\"%" PRId64 "\"}",type,v); }
static void value_f64(double v) { uint64_t bits; memcpy(&bits,&v,8); outf("{\"f64\":\"%016" PRIx64 "\"}",bits); }
static void value_f32(float v) { uint32_t bits; memcpy(&bits,&v,4); outf("{\"f32\":\"%08" PRIx32 "\"}",bits); }
static void field_count(const char *name, size_t n) { size_t mark=path_field(name); leaf_begin(); value_u64(n); leaf_end(); path_restore(mark); }
static void value_ufbx_string(ufbx_string v) { outf("{\"length\":\"%zu\",\"bytes_hex\":",v.length); hex_bytes(v.data,v.length); out("}"); }
static void value_ufbx_blob(ufbx_blob v) {
    char hash[65]; digest_hex(v.data,v.size,hash);
    outf("{\"type\":\"blob\",\"length\":\"%zu\",\"sha256\":\"%s\"}",v.size,hash);
}
static void value_vector(const double *v, size_t n, const char *type) { outf("{\"%s\":[",type); for(size_t i=0;i<n;i++) { if(i)out(","); value_f64(v[i]); } out("]}"); }
static void value_ufbx_vec2(ufbx_vec2 v) { value_vector(v.v,2,"vec2"); }
static void value_ufbx_vec3(ufbx_vec3 v) { value_vector(v.v,3,"vec3"); }
static void value_ufbx_vec4(ufbx_vec4 v) { value_vector(v.v,4,"vec4"); }
static void value_ufbx_quat(ufbx_quat v) { value_vector(v.v,4,"quat"); }
static void value_ufbx_matrix(ufbx_matrix v) { value_vector(v.v,12,"matrix"); }

static size_t dom_ordinal(const ufbx_dom_node *node) {
    // Pointer order is used only for lookup, never serialization or identities.
    for(size_t i=0;i<num_dom;i++) if(dom_entries[i].node==node) return dom_entries[i].ordinal;
    fail("DOM reference outside retained tree"); return 0;
}
static void value_dom_ref(const ufbx_dom_node *node) {
    if (!node) { out("null"); return; }
    outf("{\"dom_ordinal\":\"%zu\"}",dom_ordinal(node));
}
static const char *element_key(const ufbx_element *e) {
    if (!e) return NULL;
    if (e->element_id>=num_elements) fail("element id out of bounds");
    // Evaluated scenes keep the dense-to-source correspondence.
    return element_entries[e->element_id].key;
}
static void value_ref(const ufbx_element *e) { const char *key=element_key(e); if(key) { out("{\"ref\":"); json_string(key); out("}"); } else out("null"); }
static void value_handle(uint32_t id, bool node) {
    if(id==UFBX_NO_INDEX) { out("null");return; }
    if(!scene)fail("nonempty reference in load options");
    if(node) { if(id>=scene->nodes.count)fail("node reference out of bounds");value_ref(&scene->nodes.data[id]->element); }
    else { if(id>=num_elements)fail("element reference out of bounds");value_ref(element_entries[id].element); }
}
static int connection_order(const void *va, const void *vb);
static int anim_prop_order(const void *va, const void *vb);
static int prop_override_order(const void *va, const void *vb);
static int transform_override_order(const void *va, const void *vb);
static void *stable_sorted_copy(const void *data, size_t count, size_t size, int (*compare)(const void*,const void*)) {
    if(size && count>SIZE_MAX/size)fail("lookup sort size overflow");
    char *a=allocate(count*size),*b=allocate(count*size);memcpy(a,data,count*size);
    for(size_t width=1;width<count;) {
        for(size_t start=0;start<count;start+=width*2) {
            size_t mid=start+width<count?start+width:count,end=mid+width<count?mid+width:count;
            size_t left=start,right=mid;
            for(size_t i=start;i<end;i++) {
                charge();size_t next;
                if(right==end || (left<mid && compare(a+left*size,a+right*size)<=0))next=left++;
                else next=right++;
                memcpy(b+i*size,a+next*size,size);
            }
        }
        char *tmp=a;a=b;b=tmp;if(width>count/2)break;width*=2;
    }
    free(b);return a;
}

#include "generated.inc"

static void section_begin(const char *name) {
    if(hash_section) fail("nested section");
    outf("{\"kind\":\"begin\",\"name\":\"%s\"}\n",name);
    if(!EVP_DigestInit_ex(section_hash,EVP_sha256(),NULL)) fail("SHA-256 init");
    hash_section=true; section_count=0; path_len=0; path[0]=0; path_field(name);
}
static void section_end(const char *name) {
    char hash[65]; hash_section=false; finish_hash(section_hash,hash);
    outf("{\"kind\":\"section\",\"name\":\"%s\",\"count\":\"%" PRIu64 "\",\"sha256\":\"%s\"}\n",name,section_count,hash);
}
static bool deny_open(void *user, ufbx_stream *stream, const char *name, size_t len, const ufbx_open_file_info *info) {
    (void)user; (void)stream; (void)name; (void)len; (void)info; return false;
}
static void index_dom(const ufbx_dom_node *node, size_t depth) {
    charge(); if(depth>256) fail("DOM depth limit");
    if(num_dom==cap_dom) { size_t next=cap_dom?cap_dom*2:1024; void *p=realloc(dom_entries,next*sizeof(dom_entry)); if(!p)fail("DOM index allocation"); dom_entries=p; cap_dom=next; }
    dom_entries[num_dom]=(dom_entry){node,num_dom}; num_dom++;
    for(size_t i=0;i<node->children.count;i++) index_dom(node->children.data[i],depth+1);
}
static char *hex_key(ufbx_string s) {
    if(s.length>(SIZE_MAX-1)/2)fail("name size overflow");
    char *p=allocate(s.length*2+1); for(size_t i=0;i<s.length;i++)sprintf(p+i*2,"%02x",(unsigned char)s.data[i]); p[s.length*2]=0; return p;
}
static int legacy_component_order(const char **ap, const char **bp) {
    const char *a=*ap,*b=*bp,*ae=strchr(a,':'),*be=strchr(b,':');
    if(!ae || !be)fail("malformed legacy identity component");
    size_t an=(size_t)(ae-a),bn=(size_t)(be-b),n=an<bn?an:bn;
    int order=memcmp(a,b,n);*ap=ae+1;*bp=be+1;
    return order?order:an==bn?0:an<bn?-1:1;
}
static int element_order(const void *va, const void *vb) {
    const element_entry *a=*(element_entry*const*)va, *b=*(element_entry*const*)vb;
    if(scene->metadata.version>=7000) {
        if(a->source_id!=b->source_id)return a->source_id<b->source_id?-1:1;
        if(a->ordinal!=b->ordinal)return a->ordinal<b->ordinal?-1:1;
    }
    if(scene->metadata.version<7000 && !strncmp(a->key,"legacy:",7) && !strncmp(b->key,"legacy:",7)) {
        const char *ak=a->key+7,*bk=b->key+7;
        int order=legacy_component_order(&ak,&bk);if(order)return order;
        order=legacy_component_order(&ak,&bk);if(order)return order;
        return a->ordinal==b->ordinal?0:a->ordinal<b->ordinal?-1:1;
    }
    return strcmp(a->key,b->key);
}
static int signed_id_order(const void *va, const void *vb) {
    int64_t a=*(const int64_t*)va,b=*(const int64_t*)vb;return a==b?0:a<b?-1:1;
}
static size_t duplicate_source_ids(void) {
    if(scene->metadata.version<7000)return 0;
    for(size_t i=0;i<scene->dom_root->children.count;i++) {
        const ufbx_dom_node *objects=scene->dom_root->children.data[i];
        if(objects->name.length!=7 || memcmp(objects->name.data,"Objects",7))continue;
        int64_t *ids=allocate(objects->children.count*sizeof(int64_t));size_t count=0;
        for(size_t j=0;j<objects->children.count;j++) {
            const ufbx_dom_node *object=objects->children.data[j];
            if(object->values.count && object->values.data[0].type==UFBX_DOM_VALUE_NUMBER)ids[count++]=object->values.data[0].value_int;
        }
        qsort(ids,count,sizeof(int64_t),signed_id_order);size_t duplicates=0;
        for(size_t j=1;j<count;j++)if(ids[j]==ids[j-1])duplicates++;
        free(ids);return duplicates;
    }
    return 0;
}
static void index_elements(void) {
    num_elements=scene->elements.count;
    element_entries=allocate(num_elements*sizeof(element_entry)); memset(element_entries,0,num_elements*sizeof(element_entry));
    ordered_elements=allocate(num_elements*sizeof(element_entry*));
    for(size_t i=0;i<num_elements;i++) {
        const ufbx_element *e=scene->elements.data[i];
        if(e->element_id>=num_elements || element_entries[e->element_id].element) fail("duplicate dense element ID");
        element_entry *entry=&element_entries[e->element_id]; entry->element=e;
        entry->ordinal=e->dom_node?dom_ordinal(e->dom_node):SIZE_MAX;
        entry->source_id=e->dom_node && e->dom_node->values.count ? e->dom_node->values.data[0].value_int : INT64_MIN;
        if(e->dom_node) {
            // Legacy Model and Take records can yield multiple typed elements.
            // The primary representation owns the source key; the others are
            // explicit synthetic attachments/layers even though they share DOM.
            const ufbx_element *primary=e;
            for(size_t j=0;j<num_elements;j++) {
                const ufbx_element *candidate=scene->elements.data[j];
                if(candidate->dom_node==e->dom_node && (candidate->type<primary->type || (candidate->type==primary->type && candidate->element_id<primary->element_id)))primary=candidate;
            }
            if(primary!=e)continue;
        }
        char key[256];
        if(e==&scene->root_node->element) snprintf(key,sizeof(key),"root");
        else if(e->dom_node && scene->metadata.version>=7000) snprintf(key,sizeof(key),"id:%" PRId64 ":ordinal:%zu",entry->source_id,entry->ordinal);
        else if(e->dom_node) {
            ufbx_string type=e->dom_node->name, name=e->name;
            if(e->dom_node->values.count) {
                const ufbx_dom_value *v=&e->dom_node->values.data[0];name=v->value_str;
                if(v->value_blob.size)name=(ufbx_string){(const char*)v->value_blob.data,v->value_blob.size};
            }
            char *a=hex_key(type), *b=hex_key(name); size_t n=strlen(a)+strlen(b)+100;
            entry->key=allocate(n); snprintf(entry->key,n,"legacy:%s:%s:ordinal:%020zu",a,b,entry->ordinal); free(a);free(b);
        } else continue;
        if(!entry->key) entry->key=strdup(key);
        if(!entry->key)fail("identity allocation");
    }
    // Synthetic clones/helpers derive identity from an owner and role, with a
    // stable ordinal within that role. Dense IDs are never serialized.
    for(size_t pass=0;pass<num_elements;pass++) {
        bool pending=false, progress=false;
        for(size_t i=0;i<num_elements;i++) {
            element_entry *entry=&element_entries[i]; if(entry->key)continue;
            const ufbx_element *e=entry->element, *owner=NULL; const char *role="generated";
            for(size_t j=0;j<num_elements && !owner;j++) {
                const ufbx_element *candidate=element_entries[j].element;
                if(candidate->type==UFBX_ELEMENT_NODE) {
                    const ufbx_node *node=(const ufbx_node*)candidate;
                    if(node->geometry_transform_helper && &node->geometry_transform_helper->element==e) { owner=candidate; role="geometry-helper"; }
                    else if(node->scale_helper && &node->scale_helper->element==e) { owner=candidate; role="scale-helper"; }
                    else for(size_t a=0;a<node->all_attribs.count;a++) if(node->all_attribs.data[a]==e) { owner=candidate; role="attachment"; break; }
                }
            }
            if(!owner && e->type==UFBX_ELEMENT_NODE) { const ufbx_node *node=(const ufbx_node*)e; if(node->parent) { owner=&node->parent->element;role="child"; } }
            if(!owner && e->type==UFBX_ELEMENT_ANIM_LAYER) {
                for(size_t a=0;a<scene->anim_stacks.count && !owner;a++) {
                    const ufbx_anim_stack *stack=scene->anim_stacks.data[a];
                    for(size_t l=0;l<stack->layers.count;l++)if(&stack->layers.data[l]->element==e) { owner=&stack->element;role="take-layer";break; }
                }
            }
            if(!owner && e->type==UFBX_ELEMENT_ANIM_VALUE) {
                for(size_t a=0;a<scene->anim_layers.count && !owner;a++) {
                    const ufbx_anim_layer *layer=scene->anim_layers.data[a];
                    for(size_t v=0;v<layer->anim_values.count;v++)if(&layer->anim_values.data[v]->element==e) { owner=&layer->element;role="animated-value";break; }
                }
            }
            if(!owner && e->type==UFBX_ELEMENT_ANIM_CURVE) {
                for(size_t a=0;a<scene->anim_values.count && !owner;a++) {
                    const ufbx_anim_value *value=scene->anim_values.data[a];
                    for(size_t c=0;c<3;c++)if(value->curves[c] && &value->curves[c]->element==e) { owner=&value->element;role="component-curve";break; }
                }
            }
            if(!owner) { owner=&scene->root_node->element; role="scene-generated"; }
            const char *owner_key=element_entries[owner->element_id].key;
            if(!owner_key) { pending=true;continue; }
            size_t n=strlen(owner_key)+128; char *prefix=allocate(n);
            snprintf(prefix,n,"synthetic:%s:role:%s:type:%d:ordinal:",owner_key,role,(int)e->type);
            size_t ordinal=0;
            for(size_t j=0;j<i;j++) if(element_entries[j].key && !strncmp(element_entries[j].key,prefix,strlen(prefix)))ordinal++;
            entry->key=allocate(n);snprintf(entry->key,n,"%s%zu",prefix,ordinal);free(prefix);progress=true;
        }
        if(!pending)break;
        if(!progress)fail("synthetic owner cycle");
    }
    for(size_t i=0;i<num_elements;i++) { if(!element_entries[i].key)fail("missing identity"); ordered_elements[i]=&element_entries[i]; }
    qsort(ordered_elements,num_elements,sizeof(element_entry*),element_order);
}

static bool name_equal(ufbx_string a, const char *b) { return a.length==strlen(b) && !memcmp(a.data,b,a.length); }
static void dump_dom_value(const ufbx_dom_value *v) {
    size_t mark=path_field("type"); leaf_begin(); value_enum("ufbx_dom_value_type",v->type);leaf_end();path_restore(mark);
    mark=path_field("value_str");leaf_begin();value_ufbx_string(v->value_str);leaf_end();path_restore(mark);
    mark=path_field("value_int");leaf_begin();value_i64(v->value_int);leaf_end();path_restore(mark);
    mark=path_field("value_float");leaf_begin();value_f64(v->value_float);leaf_end();path_restore(mark);
    // The DOM exposes decoded numeric arrays in native byte order. Serialize
    // every element through memcpy, preserving width/bits on every host.
    size_t stride=0;
    switch(v->type) {
#if !CONVERTED_PROFILE
    case UFBX_DOM_VALUE_ARRAY_I8: stride=1;break;
#endif
    case UFBX_DOM_VALUE_ARRAY_I32: case UFBX_DOM_VALUE_ARRAY_F32: stride=4;break;
    case UFBX_DOM_VALUE_ARRAY_I64: case UFBX_DOM_VALUE_ARRAY_F64: stride=8;break;
    default: break;
    }
    mark=path_field("array");
    if(v->type ==
#if CONVERTED_PROFILE
        UFBX_DOM_VALUE_ARRAY_BLOB
#else
        UFBX_DOM_VALUE_ARRAY_RAW_STRING
#endif
    ) {
        // These buffers contain (pointer,length) entries, not byte payloads.
        // Never hash or serialize the pointers/padding in their backing array.
        if(v->value_blob.size%sizeof(ufbx_blob))fail("DOM blob-array size");
        size_t count=v->value_blob.size/sizeof(ufbx_blob);field_count("length",count);
        for(size_t b=0;b<count;b+=4096) {
            size_t m=path_index(b);leaf_begin();out("[");
            for(size_t i=b;i<count && i-b<4096;i++) {
                ufbx_blob blob;memcpy(&blob,(const char*)v->value_blob.data+i*sizeof(blob),sizeof(blob));
                if(i!=b)out(",");
                outf("{\"length\":\"%zu\",\"bytes_hex\":",blob.size);hex_bytes(blob.data,blob.size);out("}");
            }
            out("]");leaf_end();path_restore(m);
        }
    } else if(stride) {
        if(v->value_blob.size%stride)fail("DOM array byte size");
        size_t count=v->value_blob.size/stride;field_count("length",count);
        for(size_t b=0;b<count;b+=4096) {
            size_t m=path_index(b);leaf_begin();out("[");
            for(size_t i=b;i<count && i-b<4096;i++) {
                const unsigned char *p=(const unsigned char*)v->value_blob.data+i*stride;if(i!=b)out(",");
                switch(v->type) {
#if !CONVERTED_PROFILE
                case UFBX_DOM_VALUE_ARRAY_I8: { int8_t x;memcpy(&x,p,1);value_i64(x);break; }
#endif
                case UFBX_DOM_VALUE_ARRAY_I32: { int32_t x;memcpy(&x,p,4);value_i64(x);break; }
                case UFBX_DOM_VALUE_ARRAY_I64: { int64_t x;memcpy(&x,p,8);value_i64(x);break; }
                case UFBX_DOM_VALUE_ARRAY_F32: { float x;memcpy(&x,p,4);value_f32(x);break; }
                case UFBX_DOM_VALUE_ARRAY_F64: { double x;memcpy(&x,p,8);value_f64(x);break; }
                default: fail("DOM array type");
                }
            }
            out("]");leaf_end();path_restore(m);
        }
    } else {
        leaf_begin();value_ufbx_blob(v->value_blob);leaf_end();
        // Retain byte chunks for local first-byte diagnostics, including media.
        for(size_t b=0;b<v->value_blob.size;b+=4096) {
            size_t m=path_index(b), n=v->value_blob.size-b;if(n>4096)n=4096;
            leaf_begin();out("{\"bytes_hex\":");hex_bytes((const char*)v->value_blob.data+b,n);out("}");leaf_end();path_restore(m);
        }
    }
    path_restore(mark);
}
static void dump_dom(const ufbx_dom_node *node, size_t depth) {
    if(depth>256)fail("DOM serialization depth");
    // CreationTimeStamp and source-file paths are explicitly nonsemantic.
    if(name_equal(node->name,"CreationTimeStamp"))return;
    field_count("ordinal",dom_ordinal(node));
    size_t m=path_field("name");leaf_begin();value_ufbx_string(node->name);leaf_end();path_restore(m);
    m=path_field("values");field_count("length",node->values.count);
    for(size_t i=0;i<node->values.count;i++) { size_t n=path_index(i);dump_dom_value(&node->values.data[i]);path_restore(n); }path_restore(m);
    m=path_field("children");field_count("length",node->children.count);
    for(size_t i=0;i<node->children.count;i++) { size_t n=path_index(i);dump_dom(node->children.data[i],depth+1);path_restore(n); }path_restore(m);
}

static const char *domain(ufbx_element_type type) {
    switch(type) {
    case UFBX_ELEMENT_NODE: case UFBX_ELEMENT_BONE: case UFBX_ELEMENT_EMPTY: return "graph";
    case UFBX_ELEMENT_MESH: return "meshes";
    case UFBX_ELEMENT_SKIN_DEFORMER: case UFBX_ELEMENT_SKIN_CLUSTER: case UFBX_ELEMENT_POSE: return "skins";
    case UFBX_ELEMENT_BLEND_DEFORMER: case UFBX_ELEMENT_BLEND_CHANNEL: case UFBX_ELEMENT_BLEND_SHAPE:return "blend_shapes";
    case UFBX_ELEMENT_MATERIAL:case UFBX_ELEMENT_TEXTURE:case UFBX_ELEMENT_VIDEO:case UFBX_ELEMENT_SHADER:case UFBX_ELEMENT_SHADER_BINDING:return "materials";
    case UFBX_ELEMENT_LIGHT:case UFBX_ELEMENT_CAMERA:return "cameras_lights";
    case UFBX_ELEMENT_ANIM_STACK:case UFBX_ELEMENT_ANIM_LAYER:case UFBX_ELEMENT_ANIM_VALUE:case UFBX_ELEMENT_ANIM_CURVE:return "curves";
    default:return "advanced";
    }
}
static void dump_domain(const char *name) {
    section_begin(name);size_t count=0;
    for(size_t i=0;i<num_elements;i++)if(!strcmp(domain(ordered_elements[i]->element->type),name))count++;
    field_count("length",count);
    for(size_t i=0;i<num_elements;i++) {
        const element_entry *entry=ordered_elements[i];if(strcmp(domain(entry->element->type),name))continue;
        size_t m=path_field(entry->key);dump_element(entry->element);path_restore(m);
    }
    if(!strcmp(name,"materials")) { size_t m=path_field("texture_files");dump_ufbx_texture_file_list(&scene->texture_files);path_restore(m); }
    section_end(name);
}
static int connection_order(const void *va, const void *vb) {
    const ufbx_connection *a=va,*b=vb;
    int n=strcmp(element_key(a->src),element_key(b->src));if(n)return n;
    n=strcmp(element_key(a->dst),element_key(b->dst));if(n)return n;
    size_t len=a->src_prop.length<b->src_prop.length?a->src_prop.length:b->src_prop.length;
    n=memcmp(a->src_prop.data,b->src_prop.data,len);if(n)return n;
    if(a->src_prop.length!=b->src_prop.length)return a->src_prop.length<b->src_prop.length?-1:1;
    len=a->dst_prop.length<b->dst_prop.length?a->dst_prop.length:b->dst_prop.length;
    n=memcmp(a->dst_prop.data,b->dst_prop.data,len);if(n)return n;
    return a->dst_prop.length==b->dst_prop.length?0:a->dst_prop.length<b->dst_prop.length?-1:1;
}
static int byte_string_order(ufbx_string a, ufbx_string b) {
    size_t length=a.length<b.length?a.length:b.length;int n=memcmp(a.data,b.data,length);if(n)return n;
    return a.length==b.length?0:a.length<b.length?-1:1;
}
static int anim_prop_order(const void *va, const void *vb) {
    const ufbx_anim_prop *a=va,*b=vb;int n=strcmp(element_key(a->element),element_key(b->element));
    return n?n:byte_string_order(a->prop_name,b->prop_name);
}
static int prop_override_order(const void *va, const void *vb) {
    const ufbx_prop_override *a=va,*b=vb;int n=strcmp(element_key(element_entries[a->element_id].element),element_key(element_entries[b->element_id].element));
    return n?n:byte_string_order(a->prop_name,b->prop_name);
}
static int transform_override_order(const void *va, const void *vb) {
    const ufbx_transform_override *a=va,*b=vb;
    return strcmp(element_key(&scene->nodes.data[a->node_id]->element),element_key(&scene->nodes.data[b->node_id]->element));
}

static void dump_derived(void);
static ufbx_load_opts load_options(void) {
    ufbx_load_opts opts={0};
    opts.file_format=UFBX_FILE_FORMAT_FBX;opts.retain_dom=true;
    opts.open_file_cb.fn=deny_open;opts.load_external_files=false;
    opts.temp_allocator.memory_limit=(size_t)4<<30;opts.result_allocator.memory_limit=(size_t)4<<30;
    opts.temp_allocator.allocation_limit=1<<24;opts.result_allocator.allocation_limit=1<<24;
    if(CONVERTED_PROFILE) {
        opts.target_axes=ufbx_axes_right_handed_y_up;opts.target_unit_meters=1.0;
        opts.target_camera_axes=ufbx_axes_right_handed_y_up;opts.target_light_axes=ufbx_axes_right_handed_y_up;
        opts.space_conversion=UFBX_SPACE_CONVERSION_MODIFY_GEOMETRY;
        opts.geometry_transform_handling=UFBX_GEOMETRY_TRANSFORM_HANDLING_MODIFY_GEOMETRY;
        opts.inherit_mode_handling=UFBX_INHERIT_MODE_HANDLING_COMPENSATE;
        opts.handedness_conversion_axis=UFBX_MIRROR_AXIS_Z;
        opts.generate_missing_normals=true;opts.normalize_normals=true;
    }
    return opts;
}
static void init_output(void) {
    file_hash=EVP_MD_CTX_new();section_hash=EVP_MD_CTX_new();if(!file_hash || !section_hash || !EVP_DigestInit_ex(file_hash,EVP_sha256(),NULL))fail("hash allocation");
}
int main(int argc, char **argv) {
    if(argc!=2) { fprintf(stderr,"usage: fbx-canon FILE | --describe-options\n");return 2; }
    ufbx_load_opts opts=load_options();
    if(ufbx_source_version!=UFBX_HEADER_VERSION || ufbx_source_version!=(CONVERTED_PROFILE?23001u:11000u))fail("linked oracle version mismatch");
    if(!strcmp(argv[1],"--describe-options")) {
        init_output();section_begin("options");dump_ufbx_load_opts(&opts);section_end("options");
        if(fflush(stdout) || ferror(stdout))return 6;
        EVP_MD_CTX_free(file_hash);EVP_MD_CTX_free(section_hash);return 0;
    }
    FILE *file=fopen(argv[1],"rb");if(!file || fseek(file,0,SEEK_END))return 3;
    long length=ftell(file);if(length<0 || (uint64_t)length>(UINT64_C(2)<<30) || fseek(file,0,SEEK_SET))return 3;
    void *data=allocate((size_t)length);if(fread(data,1,(size_t)length,file)!=(size_t)length || fclose(file))return 3;
    char input_hash[65];digest_hex(data,(size_t)length,input_hash);
    char filename_hash[65];digest_hex(argv[1],strlen(argv[1]),filename_hash);
    ufbx_error error={0};scene=ufbx_load_memory(data,(size_t)length,&opts,&error);
    if(!scene) { fprintf(stderr,"ufbx error %d: %.*s\n",(int)error.type,(int)error.description.length,error.description.data);free(data);return 4; }
    if(!scene->dom_root)fail("retain_dom did not produce a tree");
    index_dom(scene->dom_root,0);index_elements();
    init_output();
    outf("{\"kind\":\"receipt\",\"schema\":\"fbx-scene-canon/1\",\"profile\":\"%s\",\"source_sha256\":\"%s\",\"header_sha256\":\"%s\",",PROFILE,SOURCE_HASH,HEADER_HASH);
    outf("\"coverage_sha256\":\"%s\",\"dumper_sha256\":\"%s\",\"input_sha256\":\"%s\",",COVERAGE_HASH,DUMPER_HASH,input_hash);
    outf("\"input_length\":\"%ld\",\"filename_sha256\":\"%s\",\"exit_status\":\"0\",\"real\":\"f64\",\"external_open\":\"denied\",",length,filename_hash);
    out("\"compiler\":");json_string(COMPILER_ID);out(",\"target\":");json_string(TARGET_TRIPLE);
    out(",\"flags\":");json_string(BUILD_FLAGS);out(",\"generator_sha256\":");json_string(GENERATOR_HASH);out("}\n");
    section_begin("options");dump_ufbx_load_opts(&opts);section_end("options");
    section_begin("globals");size_t m=path_field("oracle_dom_policy");leaf_begin();
    out("{\"scalar_width\":\"NUMBER folds original scalar width\",\"sanitization\":\"exposed oracle bits exact; discarded source bits bound by input SHA-256\",\"string_arrays\":\"pointer-backed entries decoded as byte strings\"}");leaf_end();path_restore(m);
    m=path_field("settings");dump_ufbx_scene_settings(&scene->settings);path_restore(m);
    m=path_field("metadata");dump_ufbx_metadata(&scene->metadata);path_restore(m);
    m=path_field("animation");dump_ufbx_anim(scene->anim);path_restore(m);section_end("globals");
    section_begin("dom");dump_dom(scene->dom_root,0);section_end("dom");
    section_begin("elements");field_count("length",num_elements);
    for(size_t i=0;i<num_elements;i++) { const element_entry *entry=ordered_elements[i];m=path_index(i);leaf_begin();value_ref(entry->element);leaf_end();field_count("source_ordinal",entry->ordinal);if(entry->element->dom_node && scene->metadata.version>=7000) { size_t n=path_field("source_id");leaf_begin();value_i64(entry->source_id);leaf_end();path_restore(n); }path_restore(m); }
    field_count("duplicate_source_ids",duplicate_source_ids());section_end("elements");
    section_begin("connections");ufbx_connection_list connections=scene->connections_src;
    ufbx_connection *sorted=allocate(connections.count*sizeof(ufbx_connection));memcpy(sorted,connections.data,connections.count*sizeof(ufbx_connection));qsort(sorted,connections.count,sizeof(ufbx_connection),connection_order);
    connections.data=sorted;dump_ufbx_connection_list(&connections);free(sorted);section_end("connections");
    const char *domains[]={"graph","meshes","skins","blend_shapes","materials","cameras_lights","curves","advanced"};
    for(size_t i=0;i<sizeof(domains)/sizeof(domains[0]);i++)dump_domain(domains[i]);
    dump_derived();
    char final_hash[65];finish_hash(file_hash,final_hash);hash_file=false;
    for(size_t i=0;i<num_elements;i++)free(element_entries[i].key);
    free(element_entries);free(ordered_elements);free(dom_entries);ufbx_free_scene((ufbx_scene*)scene);free(data);
    EVP_MD_CTX_free(file_hash);EVP_MD_CTX_free(section_hash);
    // Publish the completion marker only after all oracle work/cleanup succeeds.
    outf("{\"kind\":\"footer\",\"records\":\"%" PRIu64 "\",\"sha256\":\"%s\",\"exit_status\":\"0\"}\n",total_records,final_hash);
    if(fflush(stdout) || ferror(stdout))return 6;
    return 0;
}

// Evaluated leaves have a separate, closed schema. The comparator recognizes
// these paths explicitly; a numeric-looking authored field never gets epsilon.
static void eval_numbers(const double *v, size_t n, bool quaternion) {
    leaf_begin();out(quaternion?"{\"eval_quat\":[":"{\"eval_f64\":[");
    for(size_t i=0;i<n;i++) { if(!isfinite(v[i]))fail("nonfinite evaluated value (no fixture exception declared)");if(i)out(",");uint64_t bits;memcpy(&bits,&v[i],8);outf("\"%016" PRIx64 "\"",bits); }
    out("]}");leaf_end();
}
static void eval_scalar_field(const char *name, double v) { size_t m=path_field(name);eval_numbers(&v,1,false);path_restore(m); }
static void eval_matrix_field(const char *name, ufbx_matrix v) { size_t m=path_field(name);eval_numbers(v.v,12,false);path_restore(m); }
static void eval_transform_field(const char *name, ufbx_transform v) {
    size_t m=path_field(name),n=path_field("translation");eval_numbers(v.translation.v,3,false);path_restore(n);
    n=path_field("rotation");eval_numbers(v.rotation.v,4,true);path_restore(n);
    n=path_field("scale");eval_numbers(v.scale.v,3,false);path_restore(n);path_restore(m);
}
typedef struct { double time; const char *kind; const ufbx_anim_curve *curve; size_t index; int64_t numerator, denominator; } sample;
static sample *samples;
static size_t num_samples, sample_capacity;
static void add_sample(double time, const char *kind, const ufbx_anim_curve *curve, size_t index, int64_t numerator, int64_t denominator) {
    if(!isfinite(time))fail("nonfinite sample time");
    if(num_samples>=200000)fail("evaluation sample limit (200000 per take)");
    if(num_samples==sample_capacity) { size_t next=sample_capacity?sample_capacity*2:1024;void *p=realloc(samples,next*sizeof(sample));if(!p)fail("sample allocation");samples=p;sample_capacity=next; }
    samples[num_samples++]=(sample){time,kind,curve,index,numerator,denominator};
}
static void add_curve_samples(const ufbx_anim_curve *curve) {
    const ufbx_dom_node *ticks=NULL;
    if(curve->element.dom_node)for(size_t i=0;i<curve->element.dom_node->children.count;i++)if(name_equal(curve->element.dom_node->children.data[i]->name,"KeyTime"))ticks=curve->element.dom_node->children.data[i];
    for(size_t i=0;i<curve->keyframes.count;i++) {
        double t=curve->keyframes.data[i].time;
        add_sample(t,"key",curve,i,0,1);
        add_sample(nextafter(t,-INFINITY),"seconds-before-key",curve,i,-1,0);
        add_sample(nextafter(t,INFINITY),"seconds-after-key",curve,i,1,0);
        if(ticks && ticks->values.count && ticks->values.data[0].type==UFBX_DOM_VALUE_ARRAY_I64 && ticks->values.data[0].value_blob.size/8>i) {
            int64_t tick;memcpy(&tick,(const char*)ticks->values.data[0].value_blob.data+i*8,8);
            if(tick>INT64_MIN)add_sample((double)(tick-1)/(double)scene->metadata.ktime_second,"tick-before-key",curve,i,tick-1,scene->metadata.ktime_second);
            if(tick<INT64_MAX)add_sample((double)(tick+1)/(double)scene->metadata.ktime_second,"tick-after-key",curve,i,tick+1,scene->metadata.ktime_second);
        }
        if(i+1<curve->keyframes.count && curve->keyframes.data[i+1].time>t) {
            double end=curve->keyframes.data[i+1].time;
            for(int j=1;j<=3;j++)add_sample(t+(end-t)*((double)j/4.0),"segment-fraction",curve,i,j,4);
        }
    }
}
static void add_take_samples(const ufbx_anim *anim) {
    double begin=anim->time_begin,end=anim->time_end;
    if(!isfinite(begin)||!isfinite(end)||end<begin)fail("invalid take bounds");
    add_sample(nextafter(begin,-INFINITY),"seconds-before-take",NULL,0,-1,0);
    add_sample(nextafter(end,INFINITY),"seconds-after-take",NULL,0,1,0);
    add_sample(begin-1.0,"before-take",NULL,0,-1,1);
    add_sample(end+1.0,"after-take",NULL,0,1,1);
    double frames=ceil((end-begin)*30.0);
    if(!isfinite(frames) || frames>200000)fail("30 Hz grid work limit");
    for(int64_t i=0;i<(int64_t)frames;i++)add_sample(begin+(double)i/30.0,"grid-from-take-start",NULL,0,i,30);
    // End is included once even when the take duration is not a grid multiple.
    add_sample(end,"take-end",NULL,0,0,1);
}
static int sample_order(const void *va, const void *vb) {
    const sample *a=va,*b=vb;
    if(a->time!=b->time)return a->time<b->time?-1:1;
    // Keep a deterministic provenance representative when seconds coincide.
    uint64_t ab,bb;memcpy(&ab,&a->time,8);memcpy(&bb,&b->time,8);
    if(ab!=bb)return ab<bb?-1:1;
    const char *ak=a->curve?element_key(&a->curve->element):"",*bk=b->curve?element_key(&b->curve->element):"";
    int n=strcmp(ak,bk);if(n)return n;n=strcmp(a->kind,b->kind);if(n)return n;
    if(a->index!=b->index)return a->index<b->index?-1:1;
    if(a->numerator!=b->numerator)return a->numerator<b->numerator?-1:1;
    return a->denominator==b->denominator?0:a->denominator<b->denominator?-1:1;
}
static void unique_sample_times(void) {
    qsort(samples,num_samples,sizeof(sample),sample_order);
    size_t count=0;
    for(size_t i=0;i<num_samples;i++) {
        uint64_t a,b;memcpy(&a,&samples[i].time,8);
        if(count) { memcpy(&b,&samples[count-1].time,8);if(a==b)continue; }
        samples[count++]=samples[i];
    }
    num_samples=count;
}
static void dump_sample(const sample *s) {
    size_t m=path_field("time");leaf_begin();value_f64(s->time);leaf_end();path_restore(m);
    m=path_field("provenance");leaf_begin();out("{\"kind\":");json_string(s->kind);out(",\"curve\":");value_ref(s->curve?&s->curve->element:NULL);
    outf(",\"key_index\":\"%zu\",\"numerator\":\"%" PRId64 "\",\"denominator\":\"%" PRId64 "\"}",s->index,s->numerator,s->denominator);leaf_end();path_restore(m);
}
static void eval_property(const ufbx_anim *anim, const ufbx_anim_prop *binding, double time) {
    ufbx_prop p=ufbx_evaluate_prop_len(anim,binding->element,binding->prop_name.data,binding->prop_name.length,time);
    size_t m=path_field("binding");dump_ufbx_anim_prop(binding);path_restore(m);
    m=path_field("type");leaf_begin();value_enum("ufbx_prop_type",p.type);leaf_end();path_restore(m);
    m=path_field("flags");leaf_begin();value_enum("ufbx_prop_flags",p.flags);leaf_end();path_restore(m);
    m=path_field("value_int");leaf_begin();value_i64(p.value_int);leaf_end();path_restore(m);
    m=path_field("value_str");leaf_begin();value_ufbx_string(p.value_str);leaf_end();path_restore(m);
    m=path_field("value_blob");leaf_begin();value_ufbx_blob(p.value_blob);leaf_end();path_restore(m);
    m=path_field("components");eval_numbers(p.value_real_arr,4,false);path_restore(m);
}
static void dump_animation(const char *key, const ufbx_anim *anim) {
    num_samples=0;
    // Include all curves, not only non-muted active ones: authored interpolation
    // remains independently observable in every layer/composition configuration.
    for(size_t l=0;l<anim->layers.count;l++) {
        const ufbx_anim_layer *layer=anim->layers.data[l];
        for(size_t a=0;a<layer->anim_values.count;a++)for(size_t c=0;c<3;c++) {
            const ufbx_anim_curve *curve=layer->anim_values.data[a]->curves[c];
            if(curve)add_curve_samples(curve);
        }
    }
    add_take_samples(anim);
    unique_sample_times();
    if(num_elements && num_samples>10000000/num_elements)fail("scene evaluation work limit (10000000 element samples)");
    size_t base=path_field(key);field_count("length",num_samples);
    for(size_t s=0;s<num_samples;s++) {
        charge();size_t mark=path_index(s);const sample *sample=&samples[s];dump_sample(sample);
        for(size_t l=0;l<anim->layers.count;l++) {
            const ufbx_anim_layer *layer=anim->layers.data[l];size_t lm=path_field("layers"),li=path_index(l);
            const ufbx_anim_prop *bindings=stable_sorted_copy(layer->anim_props.data,layer->anim_props.count,sizeof(ufbx_anim_prop),anim_prop_order);
            for(size_t p=0;p<layer->anim_props.count;p++) {
                size_t pm=path_index(p),cm=path_field("layer");eval_property(layer->anim,&bindings[p],sample->time);path_restore(cm);
                cm=path_field("composed");eval_property(anim,&bindings[p],sample->time);path_restore(cm);path_restore(pm);
            }
            free((void*)bindings);
            path_restore(li);path_restore(lm);
        }
        ufbx_evaluate_opts opts={0};opts.open_file_cb.fn=deny_open;
        opts.temp_allocator.memory_limit=(size_t)768<<20;opts.result_allocator.memory_limit=(size_t)768<<20;
        opts.temp_allocator.allocation_limit=1<<24;opts.result_allocator.allocation_limit=1<<24;
        ufbx_error error={0};ufbx_scene *evaluated=ufbx_evaluate_scene(scene,anim,sample->time,&opts,&error);
        if(!evaluated)fail("ufbx_evaluate_scene failed (no partial dump is valid)");
        if(evaluated->elements.count!=num_elements)fail("evaluation changed element correspondence");
        size_t nm=path_field("nodes");field_count("length",evaluated->nodes.count);
        for(size_t n=0;n<num_elements;n++) {
            const ufbx_element *source=ordered_elements[n]->element;if(source->type!=UFBX_ELEMENT_NODE)continue;
            const ufbx_node *node=(const ufbx_node*)evaluated->elements.data[source->element_id];
            size_t k=path_field(element_key(source));
            eval_transform_field("local",node->local_transform);eval_transform_field("geometry",node->geometry_transform);
            eval_matrix_field("node_to_parent",node->node_to_parent);eval_matrix_field("node_to_world",node->node_to_world);
            eval_matrix_field("geometry_to_node",node->geometry_to_node);eval_matrix_field("geometry_to_world",node->geometry_to_world);
            path_restore(k);
        }path_restore(nm);
        nm=path_field("skins");field_count("length",evaluated->skin_clusters.count);
        for(size_t n=0;n<num_elements;n++) {
            const ufbx_element *source=ordered_elements[n]->element;if(source->type!=UFBX_ELEMENT_SKIN_CLUSTER)continue;
            const ufbx_skin_cluster *cluster=(const ufbx_skin_cluster*)evaluated->elements.data[source->element_id];size_t k=path_field(element_key(source));
            eval_matrix_field("geometry_to_world",cluster->geometry_to_world);path_restore(k);
        }path_restore(nm);
        nm=path_field("morphs");field_count("length",evaluated->blend_channels.count);
        for(size_t n=0;n<num_elements;n++) {
            const ufbx_element *source=ordered_elements[n]->element;if(source->type!=UFBX_ELEMENT_BLEND_CHANNEL)continue;
            const ufbx_blend_channel *channel=(const ufbx_blend_channel*)evaluated->elements.data[source->element_id];size_t k=path_field(element_key(source));
            eval_scalar_field("weight",channel->weight);path_restore(k);
        }path_restore(nm);
        ufbx_free_scene(evaluated);path_restore(mark);
    }
    path_restore(base);
}
static void dump_derived(void) {
    section_begin("triangles");field_count("length",scene->meshes.count);
    for(size_t e=0;e<num_elements;e++) {
        const ufbx_element *element=ordered_elements[e]->element;if(element->type!=UFBX_ELEMENT_MESH)continue;
        const ufbx_mesh *mesh=(const ufbx_mesh*)element;size_t mark=path_field(element_key(element));
        field_count("faces",mesh->faces.count);
        if(mesh->max_face_triangles>SIZE_MAX/(3*sizeof(uint32_t)))fail("triangulation size overflow");
        size_t cap=mesh->max_face_triangles*3;uint32_t *indices=allocate(cap*sizeof(uint32_t));
        for(size_t f=0;f<mesh->faces.count;f++) {
            size_t m=path_index(f);uint32_t count=ufbx_triangulate_face(indices,cap,mesh,mesh->faces.data[f]);
            if((size_t)count>cap/3)fail("triangulation output exceeds bound");
            field_count("length",(size_t)count*3);
            for(size_t b=0;b<(size_t)count*3;b+=4096) { size_t n=path_index(b);leaf_begin();out("[");for(size_t i=b;i<(size_t)count*3 && i-b<4096;i++) { if(i!=b)out(",");value_u64(indices[i]); }out("]");leaf_end();path_restore(n); }
            path_restore(m);
        }
        free(indices);path_restore(mark);
    }
    section_end("triangles");
    section_begin("curve_samples");field_count("length",scene->anim_curves.count);
    for(size_t e=0;e<num_elements;e++) {
        const ufbx_element *element=ordered_elements[e]->element;if(element->type!=UFBX_ELEMENT_ANIM_CURVE)continue;
        const ufbx_anim_curve *curve=(const ufbx_anim_curve*)element;size_t mark=path_field(element_key(element));
        num_samples=0;add_curve_samples(curve);add_take_samples(scene->anim);field_count("length",num_samples);
        for(size_t s=0;s<num_samples;s++) { size_t m=path_index(s);dump_sample(&samples[s]);eval_scalar_field("value",ufbx_evaluate_curve(curve,samples[s].time,0.0));path_restore(m); }
        path_restore(mark);
    }section_end("curve_samples");
    section_begin("evaluated");field_count("length",scene->anim_stacks.count?scene->anim_stacks.count:1);
    if(scene->anim_stacks.count) {
        for(size_t e=0;e<num_elements;e++) { const ufbx_element *element=ordered_elements[e]->element;if(element->type==UFBX_ELEMENT_ANIM_STACK)dump_animation(element_key(element),((const ufbx_anim_stack*)element)->anim); }
    } else dump_animation("default",scene->anim);
    section_end("evaluated");free(samples);
}
