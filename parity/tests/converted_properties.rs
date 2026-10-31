use fbx_rs::canon::V;
use fbx_rs::canon_scene::{project, DumpLimits, ElementType};

#[test]
fn geometry_baking_resets_stored_properties_after_using_their_values() {
    let input = br#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
GlobalSettings: { Properties70: {
 P: "UnitScaleFactor", "double", "Number", "",100
} }
Objects: {
 Geometry: 1, "Geometry::Triangle", "Mesh" {
  Vertices: *9 { a: 0,0,0,1,0,0,0,1,0 }
  PolygonVertexIndex: *3 { a: 0,1,-3 }
 }
 Model: 2, "Model::Triangle", "Mesh" { Properties70: {
  P: "GeometricTranslation", "Vector3D", "Vector", "",3,4,5
  P: "Lcl Rotation", "Lcl Rotation", "", "A",0,0,0
 } }
 AnimationStack: 3, "AnimStack::Take", "" {}
 AnimationLayer: 4, "AnimLayer::Base", "" {}
 AnimationCurveNode: 5, "AnimCurveNode::R", "" { Properties70: {
  P: "d|X", "Number", "", "A",0
 } }
 AnimationCurve: 6, "AnimCurve::X", "" {
  KeyTime: *2 { a: 0,46186158000 }
  KeyValueFloat: *2 { a: 0,10 }
  KeyAttrFlags: *1 { a: 4 }
  KeyAttrDataFloat: *4 { a: 0,0,0,0 }
  KeyAttrRefCount: *1 { a: 2 }
 }
}
Connections: { C: "OO",1,2
 C: "OO",2,0
 C: "OO",4,3
 C: "OO",5,4
 C: "OP",5,2,"Lcl Rotation"
 C: "OP",6,5,"d|X"
}
"#;
    let scene = project(input, &DumpLimits::default()).unwrap();
    let mesh = scene
        .elements
        .iter()
        .find(|e| e.ty == ElementType::Mesh)
        .unwrap();
    let Some(V::Vec3s(vertices)) = mesh.obj.get("vertices") else {
        panic!("vertices")
    };
    assert_eq!(vertices[0], [3.0, 4.0, 5.0]);
    let node = scene
        .elements
        .iter()
        .find(|e| e.source_id == Some(2))
        .unwrap();
    let Some(V::Obj(element)) = node.obj.get("element") else {
        panic!("element")
    };
    let Some(V::Obj(props)) = element.get("props") else {
        panic!("props")
    };
    let Some(V::Obj(props)) = props.get("props") else {
        panic!("list")
    };
    let Some(V::List(props)) = props.get("data") else {
        panic!("data")
    };
    let prop = props
        .iter()
        .find_map(|p| match p {
            V::Obj(p) if p.get("name") == Some(&V::Bytes(b"GeometricTranslation".to_vec())) => {
                Some(p)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(prop.get("value_int"), Some(&V::I(0)));
    assert_eq!(prop.get("value_real_arr"), Some(&V::Vec(vec![0.0; 4])));
}
