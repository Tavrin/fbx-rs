use fbx_rs::canon::V;
use fbx_rs::canon_scene::{project, DumpLimits, ElementType};

#[test]
fn legacy_switcher_siblings_and_signed_zero_ranges_follow_ufbx() {
    let input = br#"; FBX 6.1.0 project file
Objects: {
 Model: "Model::Switcher", "CameraSwitcher" { }
 Model: "Model::Child", "Null" { }
}
Connections: {
 Connect: "OO", "Model::Child", "Model::Scene"
 Connect: "OO", "Model::Switcher", "Model::Scene"
}
Takes: {
 Take: "Take" {
  LocalTime: 0,46186158000
  Model: "Model::Child" {
   Channel: "Transform" { Channel: "T" { Channel: "X" {
    KeyCount: 2
    Key: 0,-0.0,L,46186158000,0.0,L
   } } }
  }
 }
}
"#;
    let scene = project(input, &DumpLimits::default()).unwrap();
    assert_eq!(
        scene
            .elements
            .iter()
            .filter(|e| e.ty == ElementType::CameraSwitcher)
            .count(),
        1
    );
    assert!(!scene.elements.iter().any(|e| e.ty == ElementType::Unknown));
    let switcher = scene
        .elements
        .iter()
        .position(|e| e.ty == ElementType::Node && e.name == b"Switcher")
        .unwrap();
    let child = scene
        .elements
        .iter()
        .position(|e| e.ty == ElementType::Node && e.name == b"Child")
        .unwrap();
    let root = scene.elements.iter().find(|e| e.key == "root").unwrap();
    assert_eq!(
        root.obj.get("children"),
        Some(&V::List(vec![V::Ref(Some(switcher)), V::Ref(Some(child))]))
    );
    let curve = scene
        .elements
        .iter()
        .find(|e| e.ty == ElementType::AnimCurve)
        .unwrap();
    // The reference min keeps the second operand on equality, while max
    // keeps the first: both choices preserve an observable signed-zero bit.
    for (name, expected) in [("min_value", 0.0f64), ("max_value", -0.0f64)] {
        let Some(V::F(value)) = curve.obj.get(name) else {
            panic!("curve range")
        };
        assert_eq!(value.to_bits(), expected.to_bits());
    }
}
