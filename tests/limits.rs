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
