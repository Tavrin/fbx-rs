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

#[test]
fn blob_lookup_preserves_property_validation_order_and_parent_scope() {
    let input = br#"; FBX 7.4.0
Objects: {
 Model: 1, "Model::A", "Null" {
  BinaryData: 1
  Properties70: {
   P: "First", "Number", "", "", 1 { Other: { BinaryData: 1 } }
   P: "Second", "Number", "", "", 2
  }
 }
 Model: 2, "Model::B", "Null" {
  Properties70: {
   P: "Before", "Number", "", "", 1
   P: "Blob", "Blob", "", "", "bytes" { BinaryData: 1 }
  }
 }
}
"#;
    let mut doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let owners: Vec<_> = doc
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.name == b"Model").then_some(i))
        .collect();
    assert_eq!(
        scene_property::authored(&doc, owners[0], 10, 4096)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        scene_property::authored(&doc, owners[1], 10, 4096),
        Err(scene_property::Error::UnsupportedBlob)
    );
    let before = doc
        .nodes
        .iter()
        .find(|n| {
            doc.properties.get(n.properties.start).is_some_and(
                |p| matches!(p, syntax::Property::String(s) if s.as_ref() == b"Before"),
            )
        })
        .unwrap()
        .properties
        .start;
    doc.properties[before] = syntax::Property::I32(0);
    assert_eq!(
        scene_property::authored(&doc, owners[1], 10, 4096),
        Err(scene_property::Error::InvalidDom)
    );
}

#[test]
fn bare_and_signed_special_property_integer_views_match_ufbx() {
    for (token, integer, bits) in [
        ("inf", 0, 0x7ff0000000000000),
        ("NaN", 0, 0x7ff8000000000000),
        ("+inf", i64::MAX, 0x7ff0000000000000),
        ("-inf", i64::MIN, 0xfff0000000000000),
        ("-1.#IND", i64::MIN, 0xfff8000000000000),
    ] {
        let input = format!("; FBX 7.4.0\nModel: 1, \"Model::A\", \"Null\" {{ Properties70: {{ P: \"Value\", \"Number\", \"\", \"\", {token} }} }}\n");
        let doc = syntax::parse(input.as_bytes(), &syntax::Limits::default()).unwrap();
        let props = scene_property::authored(&doc, 0, 1, 4096).unwrap();
        assert_eq!(props[0].value_int, integer, "{token}");
        assert_eq!(props[0].value_real[0].to_bits(), bits, "{token}");
    }
}
