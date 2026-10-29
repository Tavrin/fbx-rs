use fbx_rs::{scene, syntax};

fn parse(s: &str) -> syntax::Document<'_> {
    syntax::parse(s.as_bytes(), &syntax::Limits::default()).unwrap()
}
const SCENE: &str = r#"; FBX 7.4.0
Objects:  {
    Model: 1, "Model::A", "Null" { }
    Model: 2, "Model::B", "Null" { }
    Deformer: 3, "Deformer::S", "Skin" { }
    Deformer: 4, "Deformer::B", "BlendShape" { }
    AnimationStack: 5, "AnimStack::A", "" { }
}
Connections:  {
    C: "OO", 1, 2
    C: "OO", 2, 0
    C: "OP", 3, 1, "Deform"
}
"#;
#[test]
fn indexes_and_reports_unsupported() {
    let doc = parse(SCENE);
    let scene = scene::index(&doc, &scene::Limits::default()).unwrap();
    assert_eq!(scene.objects.len(), 5);
    assert_eq!(scene.objects[0].parent, Some(1));
    assert_eq!(scene.connections.len(), 3);
    assert_eq!(
        scene.connections[2].kind,
        scene::ConnectionKind::Property(b"Deform")
    );
    assert_eq!(
        scene
            .unsupported
            .iter()
            .map(|x| &x.kind)
            .collect::<Vec<_>>(),
        vec![
            &scene::UnsupportedKind::Skin,
            &scene::UnsupportedKind::BlendShape,
            &scene::UnsupportedKind::AnimationStack
        ]
    );
}
#[test]
fn rejects_duplicate_missing_and_cycles() {
    let duplicate = SCENE.replace("Model: 2,", "Model: 1,");
    let doc = parse(&duplicate);
    assert_eq!(
        scene::index(&doc, &scene::Limits::default()).unwrap_err(),
        scene::Error::DuplicateId(1)
    );
    let missing = SCENE.replace("1, 2\n", "1, 42\n");
    let doc = parse(&missing);
    assert_eq!(
        scene::index(&doc, &scene::Limits::default()).unwrap_err(),
        scene::Error::MissingId(42)
    );
    let cycle = SCENE.replace("2, 0\n", "2, 1\n");
    let doc = parse(&cycle);
    assert!(matches!(
        scene::index(&doc, &scene::Limits::default()),
        Err(scene::Error::Cycle(_))
    ));
}
#[test]
fn limits_and_malformed_dom() {
    let mut doc = parse(SCENE);
    for (objects, connections, allocation, expected) in [
        (4, 3, usize::MAX, "objects"),
        (5, 2, usize::MAX, "connections"),
        (5, 3, 0, "allocation"),
    ] {
        let limits = scene::Limits {
            max_objects: objects,
            max_connections: connections,
            max_allocation: allocation,
        };
        assert_eq!(
            scene::index(&doc, &limits).unwrap_err(),
            scene::Error::LimitExceeded(expected)
        );
    }
    let limits = scene::Limits {
        max_objects: 5,
        max_connections: 3,
        max_allocation: usize::MAX,
    };
    assert!(scene::index(&doc, &limits).is_ok());
    let (mut low, mut high) = (0, 65536);
    while low < high {
        let mid = low + (high - low) / 2;
        let candidate = scene::Limits {
            max_allocation: mid,
            ..limits.clone()
        };
        if scene::index(&doc, &candidate).is_ok() {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    assert!(low > 0);
    for (amount, succeeds) in [(low - 1, false), (low, true), (low + 1, true)] {
        let candidate = scene::Limits {
            max_allocation: amount,
            ..limits.clone()
        };
        assert_eq!(scene::index(&doc, &candidate).is_ok(), succeeds);
    }
    doc.nodes[0].parent = Some(usize::MAX);
    assert_eq!(
        scene::index(&doc, &limits).unwrap_err(),
        scene::Error::InvalidDom
    );
}

#[test]
fn rejects_multiple_hierarchy_parents_and_invalid_property_ranges() {
    let both = SCENE.replace("C: \"OO\", 2, 0", "C: \"OO\", 2, 0\n    C: \"OO\", 2, 1");
    let doc = parse(&both);
    assert_eq!(
        scene::index(&doc, &scene::Limits::default()).unwrap_err(),
        scene::Error::MultipleParents(2)
    );
    let mut doc = parse(SCENE);
    doc.nodes[1].properties = usize::MAX..usize::MAX;
    assert_eq!(
        scene::index(&doc, &scene::Limits::default()).unwrap_err(),
        scene::Error::InvalidDom
    );
}

#[test]
fn reports_orphaned_p3_component_objects() {
    let source = r#"; FBX 7.4.0
Objects: {
 Deformer: 1, "Deformer::C", "Cluster" { }
 Deformer: 2, "Deformer::BC", "BlendShapeChannel" { }
 Geometry: 3, "Geometry::S", "Shape" { }
 AnimationLayer: 4, "AnimLayer::A", "" { }
 AnimationCurveNode: 5, "AnimCurveNode::N", "" { }
 AnimationCurve: 6, "AnimCurve::C", "" { }
}
Connections: { }
"#;
    let doc = parse(source);
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    assert_eq!(
        index
            .unsupported
            .iter()
            .map(|u| &u.kind)
            .collect::<Vec<_>>(),
        vec![
            &scene::UnsupportedKind::SkinCluster,
            &scene::UnsupportedKind::BlendShapeChannel,
            &scene::UnsupportedKind::ShapeGeometry,
            &scene::UnsupportedKind::AnimationLayer,
            &scene::UnsupportedKind::AnimationCurveNode,
            &scene::UnsupportedKind::AnimationCurve,
        ]
    );
}

#[test]
fn reports_every_unsupported_kind_on_the_scene() {
    let source = r#"; FBX 7.4.0
Objects: {
 Geometry: 10, "Geometry::M", "Mesh" {
  Vertices: *9 { a: 0,0,0,1,0,0,0,1,0 }
  PolygonVertexIndex: *3 { a: 0,1,-3 }
  Shape: "Smile" {
   Indexes: *1 { a: 0 }
   Vertices: *3 { a: 0,0,1 }
  }
 }
 Model: 1, "Model::A", "Mesh" { }
 Pose: 2, "Pose::P", "BindPose" { }
 Geometry: 3, "Geometry::C", "NurbsCurve" { }
 Geometry: 4, "Geometry::L", "Line" { }
 Deformer: 5, "Deformer::V", "VertexCacheDeformer" { }
 Cache: 6, "Cache::F", "" { }
 Constraint: 7, "Constraint::P", "Position From Positions" { }
 Constraint: 8, "Constraint::C", "Character" { }
 AudioLayer: 9, "AudioLayer::A", "" { }
 Audio: 11, "Audio::B", "" { }
 AnimationStack: 12, "AnimStack::T", "" { }
}
Connections: {
 C: "OO", 1, 0
 C: "OO", 10, 1
}
"#;
    use scene::UnsupportedKind::*;
    let doc = parse(source);
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let kinds: Vec<_> = index.unsupported.iter().map(|u| u.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Pose,
            CurveOrSurfaceGeometry,
            CurveOrSurfaceGeometry,
            Cache,
            Cache,
            Constraint,
            Constraint,
            Audio,
            Audio,
            AnimationStack,
            LegacyShape,
        ]
    );
    // The in-geometry shape is reported against its owning Geometry object.
    let legacy = index.unsupported.last().unwrap();
    assert_eq!(index.objects[legacy.object].id, 10);

    // The converter now supports poses and nested shapes; the index keeps
    // its compatibility tags, but the scene reports only unsupported content.
    let limits = fbx_rs::canon_scene::DumpLimits::default();
    let scene = fbx_rs::canon_scene::project(source.as_bytes(), &limits).unwrap();
    let ids: Vec<_> = scene.unsupported.iter().map(|&(k, id)| (k, id)).collect();
    assert_eq!(
        ids,
        vec![
            (CurveOrSurfaceGeometry, 3),
            (CurveOrSurfaceGeometry, 4),
            (Cache, 5),
            (Cache, 6),
            (Constraint, 7),
            (Constraint, 8),
            (Audio, 9),
            (Audio, 11),
            (AnimationStack, 12),
        ]
    );
    assert_eq!(
        scene
            .elements
            .iter()
            .filter(|e| e.ty == fbx_rs::canon_scene::ElementType::Pose)
            .count(),
        1
    );
    assert_eq!(
        scene
            .elements
            .iter()
            .filter(|e| e.ty == fbx_rs::canon_scene::ElementType::BlendShape)
            .count(),
        1
    );
}
