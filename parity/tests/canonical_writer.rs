use fbx_rs::canonical_writer::{CanonicalWriter, SECTIONS};
use serde_json::json;

#[test]
fn refuses_incomplete_and_misordered_streams() {
    let receipt = json!({"kind":"receipt","schema":"fbx-scene-canon/1"});
    let mut bytes = Vec::new();
    {
        let mut w = CanonicalWriter::new(&mut bytes, &receipt, 4096, 16384).unwrap();
        assert!(w.begin("meshes").is_err());
        w.begin("options").unwrap();
        assert!(w.field("/meshes/wrong", &true).is_err());
        w.field("/options/test", &true).unwrap();
        w.end().unwrap();
        assert!(w.finish().is_err());
    }
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("\"kind\":\"footer\""));
}
#[test]
fn complete_frame_has_expected_hashes_and_limits() {
    let receipt = json!({"kind":"receipt"});
    let mut w = CanonicalWriter::new(Vec::new(), &receipt, 4096, 16384).unwrap();
    for &section in SECTIONS {
        w.begin(section).unwrap();
        w.end().unwrap();
    }
    let bytes = w.finish().unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    assert!(text.contains("\"records\":\"0\""));
    assert!(CanonicalWriter::new(Vec::new(), &receipt, 1, 16).is_err());
}

#[test]
fn pinned_import_profile_options_section_has_reference_checksum() {
    use fbx_rs::canonical_writer::CanonicalValue;
    let options: serde_json::Value = serde_json::from_str(include_str!(
        "../../reference/options-engine-converted.json"
    ))
    .unwrap();
    let receipt = json!({"kind":"receipt"});
    let mut bytes = Vec::new();
    {
        let mut w = CanonicalWriter::new(&mut bytes, &receipt, 1 << 20, 1 << 22).unwrap();
        w.begin("options").unwrap();
        for field in options["fields"].as_array().unwrap() {
            w.field(
                field["path"].as_str().unwrap(),
                &CanonicalValue(&field["value"]),
            )
            .unwrap();
        }
        w.end().unwrap();
    }
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains(options["section_sha256"].as_str().unwrap()));
}

#[test]
fn a_partial_write_cannot_be_completed() {
    let receipt = json!({"kind":"receipt"});
    let mut out = Vec::new();
    {
        let mut w = CanonicalWriter::new(&mut out, &receipt, 80, 4096).unwrap();
        w.begin("options").unwrap();
        assert!(w.field("/options/example", &"x".repeat(100)).is_err());
        assert!(w.end().is_err());
        assert!(w.finish().is_err());
    }
    assert!(!String::from_utf8(out)
        .unwrap()
        .contains("\"kind\":\"footer\""));
}

#[test]
fn source_global_property_fields_use_p0_value_shapes() {
    use fbx_rs::{canonical_writer::write_source_global_properties, scene_property, syntax};
    let input = br#"; FBX 7.4.0
GlobalSettings: { Properties70: { P: "UnitScaleFactor", "double", "", "", 100 } }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let owner = doc
        .nodes
        .iter()
        .position(|n| n.name == b"GlobalSettings")
        .unwrap();
    let props = scene_property::authored(&doc, owner, 4, 4096).unwrap();
    let mut out = Vec::new();
    {
        let mut w =
            CanonicalWriter::new(&mut out, &json!({"kind":"partial"}), 4096, 1 << 20).unwrap();
        w.begin("options").unwrap();
        w.end().unwrap();
        w.begin("globals").unwrap();
        write_source_global_properties(&mut w, &props, 256).unwrap();
        w.end().unwrap();
        assert!(w.finish().is_err());
    }
    let fields: Vec<serde_json::Value> = out
        .split(|&b| b == b'\n')
        .filter(|v| !v.is_empty())
        .map(|v| serde_json::from_slice(v).unwrap())
        .collect();
    let find = |path: &str| &fields.iter().find(|v| v["path"] == path).unwrap()["value"];
    assert_eq!(
        find("/globals/settings/props/props/0/type"),
        &json!({"enum":"ufbx_prop_type", "i64":"3"})
    );
    assert_eq!(
        find("/globals/settings/props/props/0/value_real_arr/0"),
        &json!([{ "f64":"4059000000000000" }, { "f64":"0000000000000000" }, { "f64":"0000000000000000" }, { "f64":"0000000000000000" }])
    );
}

#[test]
fn emits_source_triangle_section_and_rejects_too_little_work() {
    use fbx_rs::{
        canonical_writer::{write_source_triangles, TriangleDumpLimits},
        scene, scene_mesh, syntax,
    };
    let input = br#"; FBX 7.4.0
Objects: { Geometry: 10, "Geometry::Tri", "Mesh" {
 Vertices: *9 { a: 0,0,0, 1,0,0, 0,1,0 }
 PolygonVertexIndex: *3 { a: 0,1,-3 }
} }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    let mut bytes = Vec::new();
    {
        let mut w =
            CanonicalWriter::new(&mut bytes, &json!({"kind":"receipt"}), 4096, 65536).unwrap();
        for &section in SECTIONS.iter().take_while(|&&s| s != "triangles") {
            w.begin(section).unwrap();
            w.end().unwrap();
        }
        write_source_triangles(&mut w, &index, &meshes, &TriangleDumpLimits::default()).unwrap();
    }
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\"path\":\"/triangles/length\",\"value\":{\"u64\":\"1\"}"));
    assert!(text.contains("[{\"u64\":\"0\"},{\"u64\":\"1\"},{\"u64\":\"2\"}]"));
    let mut w = CanonicalWriter::new(Vec::new(), &json!({"kind":"receipt"}), 4096, 65536).unwrap();
    for &section in SECTIONS.iter().take_while(|&&s| s != "triangles") {
        w.begin(section).unwrap();
        w.end().unwrap();
    }
    assert!(write_source_triangles(
        &mut w,
        &index,
        &meshes,
        &TriangleDumpLimits {
            max_work: 0,
            ..Default::default()
        }
    )
    .is_err());
    assert!(w.finish().is_err());
}

#[test]
fn embedded_import_profile_options_match_pinned_section() {
    let options: serde_json::Value = serde_json::from_str(include_str!(
        "../../reference/options-engine-converted.json"
    ))
    .unwrap();
    let mut bytes = Vec::new();
    {
        let mut w =
            CanonicalWriter::new(&mut bytes, &json!({"kind":"receipt"}), 1 << 20, 1 << 22).unwrap();
        w.import_profile_options().unwrap();
    }
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains(options["section_sha256"].as_str().unwrap()));
}
