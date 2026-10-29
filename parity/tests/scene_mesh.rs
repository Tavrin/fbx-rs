use fbx_rs::{scene, scene_mesh, syntax};

const INPUT: &[u8] = br#"; FBX 7.4.0
Objects: {
 Geometry: 10, "Geometry::Tri", "Mesh" {
  Vertices: *9 { a: 0,0,0,1,0,0,0,1,0 }
  PolygonVertexIndex: *3 { a: 0,1,-3 }
  LayerElementUV: 0 {
   Name: "uv0"
   MappingInformationType: "ByPolygonVertex"
   ReferenceInformationType: "IndexToDirect"
   UV: *6 { a: 0,0,1,0,0,1 }
   UVIndex: *3 { a: 2,1,0 }
  }
 }
 Model: 11, "Model::A", "Mesh" { }
 Model: 12, "Model::B", "Mesh" { }
}
Connections: {
 C: "OO", 10, 11
 C: "OO", 10, 12
 C: "OO", 11, 0
 C: "OO", 12, 0
}
"#;
#[test]
fn extracts_mesh_layer_and_instances() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert_eq!(meshes.meshes.len(), 1);
    assert_eq!(meshes.layers.len(), 1);
    assert_eq!(meshes.meshes[0].vertices.unwrap().len(), 9);
    assert_eq!(meshes.meshes[0].polygon_indices.unwrap().i64(2), Some(-3));
    let layer = &meshes.layers[0];
    assert_eq!(layer.kind, scene_mesh::LayerKind::Uv);
    assert_eq!(layer.name, Some(&b"uv0"[..]));
    let c = scene_mesh::CornerContext {
        polygon_vertex: 0,
        vertex: 0,
        polygon: 0,
        edge: 0,
    };
    assert_eq!(layer.direct_index(c).unwrap(), 2);
    assert_eq!(layer.direct.unwrap().f64(4), Some(0.0));
    assert_eq!(
        meshes.instances(0, &index, &doc).collect::<Vec<_>>(),
        [1, 2]
    );
}
#[test]
fn mesh_limits_are_checked() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let limits = scene_mesh::Limits {
        max_meshes: 0,
        ..Default::default()
    };
    assert!(matches!(
        scene_mesh::extract(&doc, &index, &limits),
        Err(scene::Error::LimitExceeded("meshes"))
    ));
    let limits = scene_mesh::Limits {
        max_layers: 0,
        ..Default::default()
    };
    assert!(matches!(
        scene_mesh::extract(&doc, &index, &limits),
        Err(scene::Error::LimitExceeded("layers"))
    ));
    let limits = scene_mesh::Limits {
        max_allocation: 0,
        ..Default::default()
    };
    assert!(matches!(
        scene_mesh::extract(&doc, &index, &limits),
        Err(scene::Error::LimitExceeded("allocation"))
    ));
}

#[test]
fn validates_topology_and_layer_bounds() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert_eq!(
        meshes.meshes[0].validate_topology().unwrap(),
        scene_mesh::Topology {
            vertices: 3,
            corners: 3,
            faces: 1
        }
    );
    let mut c = scene_mesh::CornerContext {
        polygon_vertex: 0,
        vertex: 0,
        polygon: 0,
        edge: 0,
    };
    assert_eq!(meshes.layers[0].direct_offset(c).unwrap(), 4);
    c.polygon_vertex = 3;
    assert!(meshes.layers[0].direct_offset(c).is_err());
    let invalid = String::from_utf8(INPUT.to_vec())
        .unwrap()
        .replace("0,1,-3", "0,1,3");
    let doc = syntax::parse(invalid.as_bytes(), &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert!(meshes.meshes[0].validate_topology().is_err());
}

#[test]
fn triangle_quad_and_explicit_ngon_boundary() {
    let input = br#"; FBX 7.4.0
Objects: { Geometry: 10, "Geometry::Q", "Mesh" {
 Vertices: *12 { a: 0,0,0, 3,0,0, 2,1,0, 0,1,0 }
 PolygonVertexIndex: *4 { a: 0,1,2,-4 }
} }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    let mesh = &meshes.meshes[0];
    let mut out = [99; 7];
    assert_eq!(mesh.triangulate_face_into(0, 4, &mut out).unwrap(), 2);
    assert_eq!(out, [0, 1, 2, 2, 3, 0, 99]);
    assert_eq!(mesh.triangulate_face_into(0, 3, &mut out).unwrap(), 1);
    assert_eq!(&out[..3], &[0, 1, 2]);
    assert_eq!(
        mesh.triangulate_face_into(0, 4, &mut out[..5]).unwrap_err(),
        scene::Error::BufferTooSmall { required: 6 }
    );
}

#[test]
fn ngon_bounds_and_output() {
    let input = br#"; FBX 7.4.0
Objects: { Geometry: 10, "Geometry::P", "Mesh" {
 Vertices: *15 { a: 0,0,0, 2,0,0, 3,1,0, 1,2,0, 0,1,0 }
 PolygonVertexIndex: *5 { a: 0,1,2,3,-5 }
} }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    let mesh = &meshes.meshes[0];
    let mut out = [99; 10];
    assert_eq!(
        mesh.triangulate_with_limits(0, 5, &mut out, &Default::default())
            .unwrap(),
        3
    );
    assert!(out[..9].iter().all(|&v| v < 5));
    assert_eq!(out[9], 99);
    let limits = scene_mesh::TriangulationLimits {
        max_work: 0,
        ..Default::default()
    };
    assert_eq!(
        mesh.triangulate_with_limits(0, 5, &mut out, &limits)
            .unwrap_err(),
        scene::Error::LimitExceeded("triangulation work")
    );
    let limits = scene_mesh::TriangulationLimits {
        max_allocation: 0,
        ..Default::default()
    };
    assert_eq!(
        mesh.triangulate_with_limits(0, 5, &mut out, &limits)
            .unwrap_err(),
        scene::Error::LimitExceeded("allocation")
    );
}

#[test]
fn numeric_arrays_preserve_endian_and_signed_indices() {
    use std::borrow::Cow;
    let ints = syntax::Property::Array {
        kind: b'i',
        count: 2,
        big_endian: true,
        bytes: Cow::Borrowed(&[0, 0, 0, 4, 255, 255, 255, 253]),
    };
    let a = scene_mesh::NumericArray(&ints);
    assert_eq!(a.i64(0), Some(4));
    assert_eq!(a.i64(1), Some(-3));
    assert_eq!(a.i64(2), None);
    let floats = syntax::Property::Array {
        kind: b'd',
        count: 1,
        big_endian: false,
        bytes: Cow::Owned(1.25f64.to_le_bytes().to_vec()),
    };
    assert_eq!(scene_mesh::NumericArray(&floats).f64(0), Some(1.25));
    let truncated = syntax::Property::Array {
        kind: b'l',
        count: 2,
        big_endian: false,
        bytes: Cow::Borrowed(&[0; 8]),
    };
    assert_eq!(scene_mesh::NumericArray(&truncated).i64(1), None);
}

#[test]
fn layer_mapping_modes_and_index_to_direct() {
    use syntax::{AsciiValue, Property};
    let indices = Property::AsciiArray(vec![
        AsciiValue::I64(4),
        AsciiValue::I64(3),
        AsciiValue::I64(2),
        AsciiValue::I64(1),
        AsciiValue::I64(0),
    ]);
    let mut layer = scene_mesh::Layer {
        mesh: 0,
        node: 0,
        kind: scene_mesh::LayerKind::Uv,
        typed_index: 0,
        name: None,
        mapping: None,
        reference: Some(scene_mesh::Reference::Direct),
        direct: None,
        indices: Some(scene_mesh::NumericArray(&indices)),
    };
    let c = scene_mesh::CornerContext {
        polygon_vertex: 4,
        vertex: 3,
        polygon: 2,
        edge: 1,
    };
    for (mapping, expected) in [
        (scene_mesh::Mapping::ByPolygonVertex, 4),
        (scene_mesh::Mapping::ByVertex, 3),
        (scene_mesh::Mapping::ByPolygon, 2),
        (scene_mesh::Mapping::ByEdge, 1),
        (scene_mesh::Mapping::AllSame, 0),
    ] {
        layer.mapping = Some(mapping);
        assert_eq!(layer.direct_index(c).unwrap(), expected);
        layer.reference = Some(scene_mesh::Reference::IndexToDirect);
        assert_eq!(layer.direct_index(c).unwrap(), 4 - expected);
        layer.reference = Some(scene_mesh::Reference::Direct);
    }
}

#[test]
fn edge_closure_and_material_slots() {
    let input = br#"; FBX 7.4.0
Objects: { Geometry: 10, "Geometry::Tri", "Mesh" {
 Vertices: *9 { a: 0,0,0, 1,0,0, 0,1,0 }
 PolygonVertexIndex: *3 { a: 0,1,-3 }
 Edges: *3 { a: 0,1,2 }
 LayerElementMaterial: 0 {
  MappingInformationType: "AllSame"
  ReferenceInformationType: "IndexToDirect"
  Materials: *1 { a: 2 }
 }
} }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert_eq!(meshes.meshes[0].edge_corners(2, 3).unwrap(), [2, 0]);
    assert_eq!(
        meshes.meshes[0].edge_corners(2, 0).unwrap_err(),
        scene::Error::LimitExceeded("edge work")
    );
    let context = scene_mesh::CornerContext {
        polygon_vertex: 1,
        vertex: 1,
        polygon: 0,
        edge: 1,
    };
    assert_eq!(meshes.layers[0].material_index(context).unwrap(), 2);
}

#[test]
fn validates_mapped_layer_references_and_work_limit() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert_eq!(meshes.validate_layer(0, 18).unwrap(), 3);
    assert_eq!(
        meshes.validate_layer(0, 17).unwrap_err(),
        scene::Error::LimitExceeded("layer work")
    );
    let invalid = String::from_utf8(INPUT.to_vec())
        .unwrap()
        .replace("2,1,0", "8,1,0");
    let doc = syntax::parse(invalid.as_bytes(), &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    assert!(meshes.validate_layer(0, 18).is_err());
}

#[test]
fn face_iterator_reports_truncated_polygon() {
    let input = String::from_utf8(INPUT.to_vec())
        .unwrap()
        .replace("0,1,-3", "0,-2,2");
    let doc = syntax::parse(input.as_bytes(), &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    let mut faces = meshes.meshes[0].faces().unwrap();
    assert_eq!(faces.next().unwrap().unwrap(), 0..2);
    assert!(faces.next().unwrap().is_err());
    assert!(faces.next().is_none());
}

#[test]
fn mesh_limit_boundaries() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    for (amount, ok) in [(0, false), (1, true), (2, true)] {
        let limits = scene_mesh::Limits {
            max_meshes: amount,
            ..Default::default()
        };
        assert_eq!(scene_mesh::extract(&doc, &index, &limits).is_ok(), ok);
        let limits = scene_mesh::Limits {
            max_layers: amount,
            ..Default::default()
        };
        assert_eq!(scene_mesh::extract(&doc, &index, &limits).is_ok(), ok);
    }
    let mut low = 0;
    let mut high = 1 << 20;
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = scene_mesh::Limits {
            max_allocation: mid,
            ..Default::default()
        };
        if scene_mesh::extract(&doc, &index, &limits).is_ok() {
            high = mid
        } else {
            low = mid + 1
        }
    }
    assert!(low > 0);
    for (amount, ok) in [(low - 1, false), (low, true), (low + 1, true)] {
        let limits = scene_mesh::Limits {
            max_allocation: amount,
            ..Default::default()
        };
        assert_eq!(scene_mesh::extract(&doc, &index, &limits).is_ok(), ok);
    }
}

#[test]
fn ngon_limit_boundaries() {
    let input = br#"; FBX 7.4.0
Objects: { Geometry: 10, "Geometry::P", "Mesh" {
 Vertices: *15 { a: 0,0,0, 2,0,0, 3,1,0, 1,2,0, 0,1,0 }
 PolygonVertexIndex: *5 { a: 0,1,2,3,-5 }
} }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let meshes = scene_mesh::extract(&doc, &index, &scene_mesh::Limits::default()).unwrap();
    let mesh = &meshes.meshes[0];
    let mut output = [0; 9];
    for (amount, ok) in [(4, false), (5, true), (6, true)] {
        let limits = scene_mesh::TriangulationLimits {
            max_face_vertices: amount,
            ..Default::default()
        };
        assert_eq!(
            mesh.triangulate_with_limits(0, 5, &mut output, &limits)
                .is_ok(),
            ok
        );
    }
    for use_work in [false, true] {
        let succeeds = |amount: usize, output: &mut [usize; 9]| {
            let mut limits = scene_mesh::TriangulationLimits::default();
            if use_work {
                limits.max_work = amount
            } else {
                limits.max_allocation = amount
            }
            mesh.triangulate_with_limits(0, 5, output, &limits).is_ok()
        };
        let mut low = 0;
        let mut high = 1 << 16;
        while low < high {
            let mid = low + (high - low) / 2;
            if succeeds(mid, &mut output) {
                high = mid
            } else {
                low = mid + 1
            }
        }
        assert!(
            low > 0
                && !succeeds(low - 1, &mut output)
                && succeeds(low, &mut output)
                && succeeds(low + 1, &mut output)
        );
    }
}
