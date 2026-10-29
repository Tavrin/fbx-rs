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
    let f = fixture().replace("; FBX 7.4.0", "; FBX 7.1.0").replace(
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

#[test]
fn oversized_sparse_indices_never_alias_vertex_zero() {
    for bad in [4294967296i64, -4294967296, i64::MAX, i64::MIN] {
        for from in ["a: 1,0 }", "a: 2,0 }"] {
            let f = fixture().replace(from, &format!("a: {bad},0 }}"));
            assert!(load(f.as_bytes(), &LoadOptions::default()).is_err());
        }
    }
}

#[test]
fn blender_full_weights_follow_the_sparse_offset_permutation() {
    let f = fixture()
        .replace("GlobalSettings:", "FBXHeaderExtension: {\n FBXHeaderVersion: 1003\n FBXVersion: 7400\n Creator: \"Blender (stable FBX IO) - 2.79 (sub 0)\"\n}\nGlobalSettings:")
        .replace("FullWeights: *2 { a: 100,50 }", "FullWeights: *2 { a: 20,80 }");
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    let shape = &s.blend_shapes()[0];
    assert_eq!(shape.offset_vertices, [0, 2]);
    assert_eq!(shape.position_offsets, [[1., 0., 0.], [0., 2., 0.]]);
    assert_eq!(shape.offset_weights, [0.8, 0.2]);
}

#[test]
fn duplicate_connections_keep_first_order_and_do_not_duplicate_influences() {
    let f = fixture().replace(" C: \"OO\",41,40", &" C: \"OO\",41,40\n".repeat(128));
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.skin_deformers()[0].clusters.len(), 1);
    assert_eq!(s.skin_deformers()[0].weights.len(), 2);
}

#[test]
fn pose_chain_uses_indexed_nodes_depths_and_parent_matrices() {
    let mut objects = String::new();
    let mut pose = String::new();
    let mut connections = String::new();
    for i in 1..=128 {
        objects.push_str(&format!("Model: {i}, \"Model::N{i}\", \"LimbNode\" {{}}\n"));
        connections.push_str(&format!("C: \"OO\",{i},{}\n", i - 1));
        // Reversed source order must still sort parent first.
        pose.insert_str(0, &format!("PoseNode: {{ Node: {i}\nMatrix: *16 {{ a: 1,0,0,0,0,1,0,0,0,0,1,0,{i},0,0,1 }}\n}}\n"));
    }
    let f = format!("; FBX 7.4.0\nGlobalSettings: {{ Properties70: {{ P: \"UnitScaleFactor\", \"double\", \"Number\", \"\", 100 }} }}\nObjects: {{\n{objects}\nPose: 1000, \"Pose::Chain\", \"BindPose\" {{ {pose} }}\n}}\nConnections: {{ {connections} }}\n");
    let limits = fbx_rs::Limits {
        max_deformer_work: 20000,
        ..Default::default()
    };
    let s = fbx_rs::load_with_limits(f.as_bytes(), &LoadOptions::default(), &limits).unwrap();
    let bones = &s.poses()[0].bone_poses;
    assert_eq!(bones.len(), 128);
    for (i, b) in bones.iter().enumerate() {
        assert_eq!(s.nodes()[b.bone_node].name(), format!("N{}", i + 1));
        assert_eq!(b.bone_to_parent.0[9], 1.0);
    }
    let limits = fbx_rs::Limits {
        max_deformer_work: 100,
        ..limits
    };
    assert!(
        fbx_rs::load_with_limits(f.as_bytes(), &LoadOptions::default(), &limits)
            .unwrap_err()
            .to_string()
            .contains("work limit")
    );
}

#[test]
fn many_static_meshes_keep_their_sources_and_deformers() {
    let mut objects = String::new();
    let mut connections = String::new();
    for i in 1..=64 {
        objects.push_str(&format!("Geometry: {i}, \"Geometry::M{i}\", \"Mesh\" {{ Vertices: *9 {{ a: 0,0,0, {i},0,0, 0,1,0 }}\nPolygonVertexIndex: *3 {{ a: 0,1,-3 }}\n}}\n"));
        // One legacy shape near the end catches parent-index mixups.
        if i == 63 {
            objects.push_str("Geometry: 1000, \"Geometry::Legacy\", \"Mesh\" { Vertices: *9 { a: 0,0,0, 1,0,0, 0,1,0 }\nPolygonVertexIndex: *3 { a: 0,1,-3 }\nShape: \"Nested\" { Indexes: *1 { a: 0 }\nVertices: *3 { a: 1,0,0 }\n}\n}\n");
        }
        connections.push_str(&format!("C: \"OO\",{i},0\n"));
    }
    let f = format!("; FBX 7.1.0\nObjects: {{ {objects} }}\nConnections: {{ {connections} }}\n");
    let s = load(f.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.meshes().len(), 65);
    assert_eq!(s.blend_shapes().len(), 1);
    for m in s.meshes() {
        assert_eq!(m.blend_deformers().len(), usize::from(m.name() == "Legacy"));
        if let Some(id @ 1..=64) = m.source_id() {
            assert_eq!(m.positions().values()[1][0], id as f64 * 0.01);
        }
    }
    // ufbx only synthesizes nested legacy shapes through version 7.1.
    let modern = f.replace("; FBX 7.1.0", "; FBX 7.4.0");
    let s = load(modern.as_bytes(), &LoadOptions::default()).unwrap();
    assert!(s.blend_shapes().is_empty());
}
