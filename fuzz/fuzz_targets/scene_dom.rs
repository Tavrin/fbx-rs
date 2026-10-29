#![no_main]
use fbx_rs::{
    scene, scene_draft, scene_mesh, scene_surface,
    syntax::{Document, Node, Property},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Complete canonical conversion of the raw input under small limits.
    let dump_limits = fbx_rs::canon_scene::DumpLimits {
        syntax: fbx_rs::syntax::Limits {
            max_depth: 16,
            max_nodes: 256,
            max_properties: 1024,
            max_array_elements: 4096,
            max_decompressed_bytes: 32768,
            max_string_length: 8192,
            max_allocation: 131072,
        },
        index: scene::Limits {
            max_objects: 256,
            max_connections: 256,
            max_allocation: 131072,
        },
        draft: scene_draft::Limits {
            max_allocation: 131072,
            index: scene::Limits {
                max_objects: 256,
                max_connections: 256,
                max_allocation: 131072,
            },
            mesh: scene_mesh::Limits {
                max_meshes: 256,
                max_layers: 256,
                max_allocation: 131072,
            },
        },
        max_properties: 1024,
        max_allocation: 131072,
        max_line: 1 << 20,
        max_output: 64 << 20,
    };
    // The producer hash only labels the receipt; a constant keeps the target fast.
    let _ = fbx_rs::canon_scene::dump(data, std::io::sink(), &dump_limits, "fuzz");
    if let Ok(mut parsed) = fbx_rs::syntax::parse(
        data,
        &fbx_rs::syntax::Limits {
            max_depth: 16,
            max_nodes: 256,
            max_properties: 1024,
            max_array_elements: 4096,
            max_decompressed_bytes: 32768,
            max_string_length: 8192,
            max_allocation: 131072,
        },
    ) {
        let draft_limits = scene_draft::Limits {
            max_allocation: 131072,
            index: scene::Limits {
                max_objects: 256,
                max_connections: 256,
                max_allocation: 131072,
            },
            mesh: scene_mesh::Limits {
                max_meshes: 256,
                max_layers: 256,
                max_allocation: 131072,
            },
        };
        let surface_limits = scene_surface::Limits {
            max_objects: 256,
            max_embedded_bytes: 32768,
            max_allocation: 131072,
        };
        let encoding = if data.starts_with(b"Kaydara FBX Binary  \x00\x1a") {
            scene_surface::Encoding::Binary
        } else {
            scene_surface::Encoding::Ascii
        };
        if let Ok(draft) =
            scene_draft::build_static(&parsed, encoding, &draft_limits, &surface_limits)
        {
            let remaining = draft_limits
                .max_allocation
                .saturating_sub(draft.retained_bytes);
            let _ = fbx_rs::scene_transform::world_matrices(
                &draft.core.index,
                &draft.core.transforms,
                remaining,
            );
            for mesh in &draft.core.meshes.meshes {
                if let Ok(faces) = mesh.faces() {
                    for face in faces.take(32).flatten() {
                        if face.len() <= 64 {
                            let mut output = [0usize; 192];
                            let limits = scene_mesh::TriangulationLimits {
                                max_face_vertices: 64,
                                max_allocation: remaining,
                                max_work: 32768,
                            };
                            let _ = mesh.triangulate_with_limits(
                                face.start,
                                face.len(),
                                &mut output,
                                &limits,
                            );
                        }
                    }
                }
            }
            for i in 0..draft.core.meshes.layers.len().min(32) {
                let _ = draft.core.meshes.validate_layer(i, 32768);
            }
        }
        if !parsed.nodes.is_empty() && !data.is_empty() {
            let i = usize::from(data[0]) % parsed.nodes.len();
            match data[0] & 3 {
                0 => parsed.nodes[i].parent = Some(usize::MAX),
                1 => parsed.nodes[i].properties = usize::MAX..usize::MAX,
                2 => parsed.nodes[i].properties = 1..0,
                _ => parsed.nodes[i].parent = Some(i),
            }
            let _ = scene_draft::build_static(&parsed, encoding, &draft_limits, &surface_limits);
        }
    }
    let mut doc = Document {
        version: 7400,
        nodes: Vec::new(),
        properties: Vec::new(),
    };
    let names: &[&[u8]] = &[
        b"Objects",
        b"Connections",
        b"Model",
        b"Deformer",
        b"C",
        b"AnimationStack",
    ];
    for chunk in data.chunks_exact(4).take(64) {
        let pos = doc.properties.len();
        doc.properties.push(Property::I64(i64::from(chunk[0])));
        let start = if chunk[1] & 0x80 == 0 {
            pos
        } else {
            usize::MAX
        };
        let end = if chunk[2] & 0x80 == 0 { pos + 1 } else { 0 };
        let parent = if chunk[3] & 0x80 == 0 {
            None
        } else {
            Some(usize::from(chunk[3] & 0x7f))
        };
        doc.nodes.push(Node {
            name: names[usize::from(chunk[1]) % names.len()],
            parent,
            properties: start..end,
        });
    }
    let _ = scene::index(
        &doc,
        &scene::Limits {
            max_objects: 64,
            max_connections: 64,
            max_allocation: 32768,
        },
    );
    let limits = scene_draft::Limits {
        max_allocation: 32768,
        index: scene::Limits {
            max_objects: 64,
            max_connections: 64,
            max_allocation: 32768,
        },
        mesh: scene_mesh::Limits {
            max_meshes: 64,
            max_layers: 64,
            max_allocation: 32768,
        },
    };
    let _ = scene_draft::build_static(
        &doc,
        scene_surface::Encoding::Ascii,
        &limits,
        &scene_surface::Limits {
            max_objects: 64,
            max_embedded_bytes: 8192,
            max_allocation: 32768,
        },
    );
});
