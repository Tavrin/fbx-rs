use fbx_rs::{scene_property, syntax};

#[test]
fn authored_properties_keep_last_duplicate_and_value_views() {
    let input = br#"; FBX 7.4.0
Objects: {
 Model: 1, "Model::A", "Null" {
  Properties70: {
   P: "Z", "Number", "", "A", 3
   P: "V", "Vector3D", "", "L3M4", 1, 2, 3
   P: "Z", "String", "", "U", "text"
  }
 }
}
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let owner = doc.nodes.iter().position(|n| n.name == b"Model").unwrap();
    let p = scene_property::authored(&doc, owner, 3, 4096).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].name, b"V");
    assert_eq!(p[0].flags, 0x1000000 | 0x400000 | 0x30 | 0x400);
    assert_eq!(p[0].value_int, 1);
    assert_eq!(p[0].value_real, [1.0, 2.0, 3.0, 0.0]);
    assert_eq!(p[1].name, b"Z");
    assert_eq!(p[1].kind, scene_property::Type::String);
    assert_eq!(p[1].flags, 0x2000000 | 2);
    assert_eq!(p[1].value_str, b"text");
    assert_eq!(
        scene_property::authored(&doc, owner, 2, 4096),
        Err(scene_property::Error::LimitExceeded)
    );
    assert_eq!(
        scene_property::authored(&doc, owner, 3, 1),
        Err(scene_property::Error::LimitExceeded)
    );
}

#[test]
fn unsupported_strings_are_explicit() {
    let input = "; FBX 7.4.0\nObjects: { Model: 1, \"Model::A\", \"Null\" { Properties70: { P: \"é\", \"Number\", \"\", \"\", 1 } } }\n";
    let doc = syntax::parse(input.as_bytes(), &syntax::Limits::default()).unwrap();
    let owner = doc.nodes.iter().position(|n| n.name == b"Model").unwrap();
    assert_eq!(
        scene_property::authored(&doc, owner, 10, 4096),
        Err(scene_property::Error::UnsupportedString)
    );
}

#[test]
fn template_selection_stops_at_first_type_and_keeps_default_list_separate() {
    let input = br#"; FBX 7.4.0
Definitions: {
 ObjectType: "NodeAttribute" { PropertyTemplate: "FbxCamera" { Properties70: { P: "FieldOfView", "Number", "", "", 45 } } }
 ObjectType: "NodeAttribute" { PropertyTemplate: "FbxLight" { Properties70: { P: "Intensity", "Number", "", "", 100 } } }
 ObjectType: "Model" { PropertyTemplate: "FbxNode" { Properties70: { P: "X", "Number", "", "", 2 } } }
}
Objects: { Model: 1, "Model::A", "Null" { Properties70: { P: "Y", "Number", "", "", 3 } } }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let camera = scene_property::template_owner(&doc, b"NodeAttribute", b"Camera")
        .unwrap()
        .unwrap();
    let properties = scene_property::authored(&doc, camera, 4, 4096).unwrap();
    assert_eq!(properties[0].name, b"FieldOfView");
    assert_eq!(
        scene_property::template_owner(&doc, b"NodeAttribute", b"Light").unwrap(),
        None
    );
    assert!(
        scene_property::template_owner(&doc, b"Model", b"AnySubtype")
            .unwrap()
            .is_some()
    );
    let owner = doc.nodes.iter().position(|n| n.name == b"Model").unwrap();
    let layers = scene_property::layers(&doc, owner, 4, 4096).unwrap();
    assert_eq!(layers.find(b"X").unwrap().value_real[0], 2.0);
    assert_eq!(layers.find(b"Y").unwrap().value_real[0], 3.0);
    assert!(layers.find(b"Z").is_none());
}
