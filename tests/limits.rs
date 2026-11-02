//! Every resource limit is reachable from the public load call, and an
//! unrecognised layer mapping mode is ignored the way ufbx ignores it.
use fbx_rs::{load_with_limits, Error, Limit, Limits, LoadOptions, Scene};

const RICH: &[u8] = include_bytes!("fixtures/allocation-rich.fbx");
const RICH_BINARY: &[u8] = include_bytes!("fixtures/allocation-rich-binary.fbx");

/// One mesh whose only polygon has `corners` corners.
fn mesh_doc(corners: usize, layers: &str) -> Vec<u8> {
    // A convex regular polygon, so every triangulation succeeds.
    let vertices = (0..corners)
        .map(|i| {
            let angle = std::f64::consts::TAU * i as f64 / corners as f64;
            format!("{:.6},{:.6},0.0", angle.cos(), angle.sin())
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut indices = (0..corners).map(|i| i.to_string()).collect::<Vec<_>>();
    indices[corners - 1] = format!("{}", -(corners as i64));
    format!(
        "; FBX 7.4.0 project file
FBXHeaderExtension: {{ FBXVersion: 7400 }}
Objects: {{
 Geometry: 1000, \"Geometry::G\", \"Mesh\" {{
  Vertices: *{} {{ a: {vertices} }}
  PolygonVertexIndex: *{corners} {{ a: {} }}
{layers}
 }}
 Model: 2000, \"Model::M\", \"Mesh\" {{ }}
}}
Connections: {{
 C: \"OO\",1000,2000
 C: \"OO\",2000,0
}}
",
        corners * 3,
        indices.join(",")
    )
    .into_bytes()
}

fn layer(kind: &str, data: &str, mapping: &str, values: &str) -> String {
    format!(
        "  LayerElement{kind}: 0 {{
   Name: \"\"
   MappingInformationType: \"{mapping}\"
   ReferenceInformationType: \"Direct\"
   {data}: {values}
  }}
"
    )
}

fn load(bytes: &[u8], limits: &Limits) -> Result<Scene, Error> {
    load_with_limits(bytes, &LoadOptions::default(), limits)
}

#[test]
fn every_limit_field_rejects_with_its_own_name() {
    let pentagon = mesh_doc(5, "");
    // (limit, input, edit): the edit lowers exactly one `Limits` field.
    type Edit = fn(&mut Limits);
    let cases: [(Limit, &[u8], Edit); 16] = [
        (Limit::Depth, RICH, |l| l.max_depth = 1),
        (Limit::Nodes, RICH, |l| l.max_nodes = 1),
        (Limit::Properties, RICH, |l| l.max_properties = 1),
        (Limit::ArrayElements, RICH, |l| l.max_array_elements = 1),
        (Limit::DecompressedBytes, RICH_BINARY, |l| {
            l.max_decompressed_bytes = 1
        }),
        (Limit::StringLength, RICH, |l| l.max_string_length = 1),
        (Limit::Objects, RICH, |l| l.max_objects = 1),
        (Limit::AnimationKeys, RICH, |l| l.max_animation_keys = 0),
        (Limit::Connections, RICH, |l| l.max_connections = 1),
        (Limit::Allocation, RICH, |l| l.max_allocation = 1),
        (Limit::DeformerWork, RICH, |l| l.max_deformer_work = 1),
        (Limit::AnimationWork, RICH, |l| l.max_animation_work = 1),
        (Limit::Meshes, RICH, |l| l.max_meshes = 0),
        (Limit::Layers, RICH, |l| l.max_layers = 0),
        (Limit::FaceVertices, &pentagon, |l| l.max_face_vertices = 4),
        (Limit::TriangulationWork, &pentagon, |l| {
            l.max_triangulation_work = 1
        }),
    ];
    for (limit, input, edit) in cases {
        load(input, &Limits::default()).expect("accepted at the default limits");
        let mut lowered = Limits::default();
        edit(&mut lowered);
        match load(input, &lowered) {
            Err(Error::LimitExceeded(hit)) => {
                assert_eq!(hit, limit, "wrong limit named: {limit:?}");
                assert!(
                    Error::LimitExceeded(hit)
                        .to_string()
                        .contains(limit.field()),
                    "{limit:?} message does not name {}",
                    limit.field()
                );
            }
            other => panic!("{limit:?}: expected LimitExceeded, got {other:?}"),
        }
    }
    let tiny = Limits {
        max_input_bytes: RICH.len() - 1,
        ..Limits::default()
    };
    assert!(matches!(
        load(RICH, &tiny),
        Err(Error::InputTooLarge { limit, .. }) if limit == RICH.len() - 1
    ));
}

#[test]
fn by_control_point_mapping_is_ignored_like_ufbx() {
    // ufbx 0.23.1 has no ByControlPoint alias: a layer element with a mapping
    // it does not know is dropped with a warning (ufbx.c, ufbxi_warn_polygon_mapping)
    // and the mesh still loads, for every layer kind that carries a mapping.
    let normals = "0.0,0.0,1.0,0.0,0.0,1.0,0.0,0.0,1.0";
    let ignored = |mapping: &str| {
        mesh_doc(
            3,
            &[
                layer(
                    "Normal",
                    "Normals",
                    mapping,
                    &format!("*9 {{ a: {normals} }}"),
                ),
                layer("UV", "UV", mapping, "*6 { a: 0,0,1,0,0,1 }"),
                layer(
                    "Color",
                    "Colors",
                    mapping,
                    "*12 { a: 1,0,0,1,0,1,0,1,0,0,1,1 }",
                ),
                layer("Material", "Materials", mapping, "*1 { a: 0 }"),
                layer("Smoothing", "Smoothing", mapping, "*1 { a: 1 }"),
            ]
            .concat(),
        )
    };
    let alias = load(&ignored("ByControlPoint"), &Limits::default()).expect("loads");
    let unknown = load(&ignored("NoSuchMapping"), &Limits::default()).expect("loads");
    let bare = load(&mesh_doc(3, ""), &Limits::default()).expect("loads");
    for scene in [&alias, &unknown] {
        let (got, want) = (&scene.meshes()[0], &bare.meshes()[0]);
        assert_eq!(got.normals(), want.normals());
        assert_eq!(got.uv_sets().len(), want.uv_sets().len());
        assert_eq!(got.color_sets().len(), want.color_sets().len());
        assert_eq!(got.triangles(), want.triangles());
    }
    // The spelled-out mapping on the same data is honoured.
    let used = load(&ignored("ByPolygonVertex"), &Limits::default()).expect("loads");
    assert!(used.meshes()[0].normals().is_some());
    assert_eq!(used.meshes()[0].uv_sets().len(), 1);
}

#[test]
fn unknown_reference_label_is_ignored_like_ufbx() {
    // ufbx never reads ReferenceInformationType (the string does not occur in
    // ufbx.c); a layer is indexed exactly when it has an index array. A label
    // that is neither Direct nor IndexToDirect must therefore not fail the mesh.
    let normals = "*9 { a: 1,0,0,0,1,0,0,0,1 }";
    let build = |label: &str, indexed: bool| {
        let index = if indexed {
            "   NormalsIndex: *3 { a: 2,1,0 }\n"
        } else {
            ""
        };
        let element = format!(
            "  LayerElementNormal: 0 {{
   Name: \"\"
   MappingInformationType: \"ByPolygonVertex\"
   ReferenceInformationType: \"{label}\"
   Normals: {normals}
{index}  }}
"
        );
        load(&mesh_doc(3, &element), &Limits::default()).expect("loads")
    };
    let direct = build("Direct", false);
    let unknown_direct = build("NoSuchReference", false);
    let indexed = build("IndexToDirect", true);
    let unknown_indexed = build("NoSuchReference", true);
    assert_eq!(
        unknown_direct.meshes()[0].normals(),
        direct.meshes()[0].normals()
    );
    assert_eq!(
        unknown_indexed.meshes()[0].normals(),
        indexed.meshes()[0].normals()
    );
    // The index array is what selects the data, not the label.
    assert_ne!(
        unknown_indexed.meshes()[0].normals(),
        unknown_direct.meshes()[0].normals()
    );
}

#[test]
fn invalid_vertex_count_loads_with_a_warning() {
    let valid = mesh_doc(3, "");
    let input = String::from_utf8(valid)
        .unwrap()
        .replace("Vertices: *9", "Vertices: *8");
    // Keep the declared count and data consistent, but not a multiple of three.
    let start = input.find("{ a: ").unwrap() + 5;
    let end = input[start..].find(" }").unwrap() + start;
    let values = input[start..end]
        .split(',')
        .take(8)
        .collect::<Vec<_>>()
        .join(",");
    let input = format!("{}{}{}", &input[..start], values, &input[end..]);
    let scene = load(input.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(scene.meshes().len(), 1);
    assert!(scene.meshes()[0].positions().values().is_empty());
    assert!(scene.warnings().iter().any(|w| w.contains("vertex count")));
}

#[test]
fn decreasing_limit_envelopes_never_substitute_empty_meshes() {
    let mut paths = std::fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "fbx"))
        .collect::<Vec<_>>();
    if let Some(manifest) = std::env::var_os("FBX_LIMIT_CORPUS_PATHS") {
        paths.extend(
            std::fs::read_to_string(manifest)
                .unwrap()
                .lines()
                .map(std::path::PathBuf::from),
        );
    }
    let edits: [fn(&mut Limits) -> &mut usize; 17] = [
        |l| &mut l.max_input_bytes,
        |l| &mut l.max_depth,
        |l| &mut l.max_nodes,
        |l| &mut l.max_properties,
        |l| &mut l.max_array_elements,
        |l| &mut l.max_decompressed_bytes,
        |l| &mut l.max_string_length,
        |l| &mut l.max_objects,
        |l| &mut l.max_animation_keys,
        |l| &mut l.max_connections,
        |l| &mut l.max_allocation,
        |l| &mut l.max_deformer_work,
        |l| &mut l.max_animation_work,
        |l| &mut l.max_meshes,
        |l| &mut l.max_layers,
        |l| &mut l.max_face_vertices,
        |l| &mut l.max_triangulation_work,
    ];
    let mut loads = 0;
    for (fixture, path) in paths.iter().enumerate() {
        let bytes = std::fs::read(path).unwrap();
        let baseline = load(&bytes, &Limits::default()).unwrap();
        for edit in edits {
            let mut limits = Limits::default();
            // Start near the fixture's size for count/byte limits, avoiding
            // repeated loads at enormous defaults that cannot constrain it.
            let mut value = (*edit(&mut limits)).min(bytes.len().saturating_mul(256).max(1));
            let mut refused = false;
            loop {
                *edit(&mut limits) = value;
                loads += 1;
                match load(&bytes, &limits) {
                    Ok(scene) => {
                        assert!(
                            !refused,
                            "non-monotonic refusal: {} at {value}",
                            path.display()
                        );
                        assert_eq!(
                            scene.warnings(),
                            baseline.warnings(),
                            "{} at {value}",
                            path.display()
                        );
                        assert_eq!(scene.meshes().len(), baseline.meshes().len());
                        for (mesh, expected) in scene.meshes().iter().zip(baseline.meshes()) {
                            assert_eq!(
                                mesh.positions(),
                                expected.positions(),
                                "{} at {value}",
                                path.display()
                            );
                            assert_eq!(mesh.triangles(), expected.triangles());
                        }
                    }
                    Err(Error::LimitExceeded(_) | Error::InputTooLarge { .. }) => refused = true,
                    Err(e) => panic!("{} at {value}: {e:?}", path.display()),
                }
                if value == 0 {
                    break;
                }
                value /= 2;
            }
        }
        if (fixture + 1) % 25 == 0 {
            eprintln!(
                "decreasing envelopes: {}/{}, {loads} loads",
                fixture + 1,
                paths.len()
            );
        }
    }
    eprintln!(
        "decreasing envelopes: {} fixtures, {loads} loads",
        paths.len()
    );
}
