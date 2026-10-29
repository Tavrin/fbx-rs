use fbx_rs::{load, LoadOptions, SkinningMethod};

fn fixture() -> String {
    r#"; FBX 7.4.0
GlobalSettings: { Properties70: { P: "UnitScaleFactor", "double", "Number", "", 100 } }
Objects: {
 Geometry: 10, "Geometry::Triangle", "Mesh" {
  Vertices: *9 { a: 0,0,0, 1,0,0, 0,1,0 }
  PolygonVertexIndex: *3 { a: 0,1,-3 }
 }
 Model: 20, "Model::Mesh", "Mesh" {}
 Model: 30, "Model::Joint", "LimbNode" {}
 NodeAttribute: 31, "NodeAttribute::Joint", "LimbNode" {}
 Deformer: 40, "Deformer::Skin", "Skin" { SkinningType: "Blend"
  Indexes: *1 { a: 1 }
  BlendWeights: *1 { a: 0.75 }
 }
 Deformer: 41, "Deformer::Cluster", "Cluster" {
  Indexes: *2 { a: 1,0 }
  Weights: *2 { a: 0.5,1 }
  Transform: *16 { a: 1,0,0,0, 0,1,0,0, 0,0,1,0, 2,0,0,1 }
  TransformLink: *16 { a: 1,0,0,0, 0,1,0,0, 0,0,1,0, 3,0,0,1 }
 }
 Deformer: 50, "Deformer::Morph", "BlendShape" {}
 Deformer: 51, "Deformer::Smile", "BlendShapeChannel" {
  DeformPercent: 75
  FullWeights: *2 { a: 100,50 }
 }
 Geometry: 52, "Geometry::Full", "Shape" {
  Indexes: *2 { a: 2,0 }
  Vertices: *6 { a: 0,2,0, 1,0,0 }
  Normals: *6 { a: 0,0,1, 0,1,0 }
 }
 Geometry: 53, "Geometry::Half", "Shape" {
  Indexes: *1 { a: 0 }
  Vertices: *3 { a: 0.5,0,0 }
 }
 Pose: 60, "Pose::Bind", "BindPose" {
  PoseNode: { Node: 30
   Matrix: *16 { a: 1,0,0,0, 0,1,0,0, 0,0,1,0, 3,0,0,1 }
  }
 }
}
Connections: {
 C: "OO",10,20
 C: "OO",20,0
 C: "OO",30,0
 C: "OO",31,30
 C: "OO",40,10
 C: "OO",41,40
 C: "OO",30,41
 C: "OO",50,10
 C: "OO",51,50
 C: "OO",52,51
 C: "OO",53,51
}
"#
    .into()
}

#[test]
fn stored_skin_bind_pose_and_in_between_targets() {
    let s = load(fixture().as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.meshes()[0].skin_deformers(), &[0]);
    assert_eq!(s.meshes()[0].blend_deformers(), &[0]);
    assert_eq!(s.bones().len(), 1);
    let node = s.nodes().iter().position(|n| n.name() == "Joint").unwrap();
    assert_eq!(s.nodes()[node].bone(), Some(0));
    assert_eq!(s.nodes()[node].bind_pose(), Some(0));
    assert_eq!(s.bones()[0].instances, [node]);
    let skin = &s.skin_deformers()[0];
    assert_eq!(skin.skinning_method, SkinningMethod::BlendedDqLinear);
    assert_eq!(skin.vertices.len(), 3);
    assert_eq!(skin.vertices[2].num_weights, 0);
    assert_eq!(skin.vertices[1].dq_weight, 0.75);
    assert_eq!(skin.weights[0].weight, 1.0);
    assert_eq!(s.skin_clusters()[0].vertices, [1, 0]);
    assert_eq!(s.skin_clusters()[0].bone_node, Some(node));
    assert_eq!(s.skin_clusters()[0].mesh_node_to_bone.0[9], 2.0);
    assert_eq!(s.skin_clusters()[0].bind_to_world.0[9], 3.0);
    assert_eq!(s.poses()[0].bone_poses[0].bone_node, node);
    let c = &s.blend_channels()[0];
    assert_eq!(c.weight, 0.75);
    assert_eq!(c.keyframes[0].target_weight, 0.5);
    assert_eq!(c.keyframes[0].shape, 1);
    assert_eq!(c.keyframes[0].effective_weight, 0.5);
    assert_eq!(c.keyframes[1].effective_weight, 0.5);
    let shape = &s.blend_shapes()[0];
    assert_eq!(shape.offset_vertices, [0, 2]);
    assert_eq!(shape.position_offsets, [[1., 0., 0.], [0., 2., 0.]]);
    assert_eq!(shape.normal_offsets, [[0., 1., 0.], [0., 0., 1.]]);
    assert!(s.unsupported().is_empty());
}

#[test]
fn malformed_deformers_return_errors() {
    let valid = fixture();
    for bad in [
        valid.replace("a: 1,0 }", "a: 9,0 }"),
        valid.replace("Weights: *2 { a: 0.5,1 }", "Weights: *1 { a: 0.5 }"),
        valid.replace("a: 2,0 }", "a: 99,0 }"),
        valid.replace(
            "Vertices: *6 { a: 0,2,0, 1,0,0 }",
            "Vertices: *3 { a: 0,2,0 }",
        ),
        valid.replace(
            "FullWeights: *2 { a: 100,50 }",
            "FullWeights: *1 { a: 100 }",
        ),
        valid.replace("BlendWeights: *1 { a: 0.75 }", "BlendWeights: *0 { a: }"),
        valid.replace(" C: \"OO\",53,51", " C: \"OO\",53,51\n C: \"OO\",50,51"),
        valid.replace("Node: 30", "Node: 999"),
    ] {
        assert!(load(bad.as_bytes(), &LoadOptions::default()).is_err());
    }
}

#[test]
fn influence_order_retains_zero_and_negative_weights() {
    let f = fixture()
        .replace(
            " Deformer: 50,",
            r#" Deformer: 42, "Deformer::Second", "Cluster" {
  Indexes: *3 { a: 0,1,2 }
  Weights: *3 { a: 1,0,-1 }
 }
 Deformer: 50,"#,
        )
        .replace(
            " C: \"OO\",40,10",
            " C: \"OO\",40,10\n C: \"OO\",42,40\n C: \"OO\",30,42",
        );
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    let skin = &s.skin_deformers()[0];
    assert_eq!(skin.max_weights_per_vertex, 2);
    // Cluster 42 is connected first, despite appearing after cluster 41.
    assert_eq!(s.skin_clusters()[skin.clusters[0]].source_id, Some(42));
    assert_eq!(
        skin.weights
            .iter()
            .map(|w| (w.cluster_index, w.weight))
            .collect::<Vec<_>>(),
        [(0, 1.), (1, 1.), (1, 0.5), (0, 0.), (0, -1.)]
    );
}

#[test]
fn negative_targets_and_dual_quaternion_defaults() {
    let f = fixture()
        .replace(
            "SkinningType: \"Blend\"",
            "SkinningType: \"DualQuaternion\"",
        )
        .replace("DeformPercent: 75", "DeformPercent: -75")
        .replace("a: 100,50", "a: -100,-50");
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.skin_deformers()[0].vertices[0].dq_weight, 1.0);
    assert_eq!(s.skin_deformers()[0].vertices[1].dq_weight, 0.75);
    let c = &s.blend_channels()[0];
    assert_eq!(c.keyframes[0].target_weight, -1.0);
    assert_eq!(c.keyframes[0].effective_weight, 0.5);
    assert_eq!(c.keyframes[1].effective_weight, 0.5);
}

#[test]
fn nested_shapes_generate_owned_channels_and_canonical_records() {
    let f = fixture().replace(
        "  PolygonVertexIndex:",
        r#"  Properties70: { P: "Legacy", "Number", "", "", 25 }
  Shape: "Legacy" {
   Indexes: *1 { a: 1 }
   Vertices: *3 { a: 0,0,2 }
  }
  PolygonVertexIndex:"#,
    );
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.blend_deformers().len(), 2);
    let c = s
        .blend_channels()
        .iter()
        .find(|c| c.name == "Legacy")
        .unwrap();
    assert_eq!(c.source_id, None);
    assert_eq!(c.weight, 0.25);
    assert_eq!(c.keyframes[0].effective_weight, 0.25);
    let shape = &s.blend_shapes()[c.keyframes[0].shape];
    assert_eq!(shape.source_id, None);
    assert_eq!(shape.offset_vertices, [1]);
    assert_eq!(shape.position_offsets, [[0., 0., 2.]]);
    let full = fbx_rs::canon_scene::project(f.as_bytes(), &Default::default()).unwrap();
    let channel = full
        .elements
        .iter()
        .find(|e| e.name == b"Legacy" && e.ty == fbx_rs::canon_scene::ElementType::BlendChannel)
        .unwrap();
    let fbx_rs::canon::V::Obj(header) = channel.obj.get("element").unwrap() else {
        panic!("header")
    };
    let fbx_rs::canon::V::Obj(props) = header.get("props").unwrap() else {
        panic!("props")
    };
    let fbx_rs::canon::V::Obj(list) = props.get("props").unwrap() else {
        panic!("list")
    };
    let fbx_rs::canon::V::List(data) = list.get("data").unwrap() else {
        panic!("data")
    };
    let fbx_rs::canon::V::Obj(prop) = &data[0] else {
        panic!("prop")
    };
    assert_eq!(prop.get("flags"), Some(&fbx_rs::canon::V::I(0x8000)));
    assert!(full.connections.iter().any(|v| matches!(v,
        fbx_rs::canon::V::Obj(o) if matches!(o.get("src_prop"),Some(fbx_rs::canon::V::Bytes(name)) if name==b"Legacy")
    )));
    assert_eq!(
        full.elements
            .iter()
            .filter(|e| e.ty == fbx_rs::canon_scene::ElementType::BlendChannel)
            .count(),
        s.blend_channels().len()
    );
}

#[test]
fn direct_blend_percent_follows_property_and_template_precedence() {
    let f = fixture().replace(
        "GlobalSettings:",
        r#"Definitions: {
 ObjectType: "Deformer" {
  PropertyTemplate: "FbxBlendShapeChannel" {
   Properties70: { P: "DeformPercent", "Number", "", "", 20 }
  }
 }
}
GlobalSettings:"#,
    );
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.blend_channels()[0].weight, 0.75);
    let f = f.replace(
        "  DeformPercent: 75",
        r#"  Properties70: { P: "Other", "Number", "", "", 1 }
  DeformPercent: 75"#,
    );
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.blend_channels()[0].weight, 0.2);
}
