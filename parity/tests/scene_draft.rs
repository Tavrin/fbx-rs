use fbx_rs::{scene, scene_draft, syntax};
#[test]
fn aggregate_budget_covers_dom_and_conversion() {
    let input = br#"; FBX 7.4.0
Objects: { Model: 1, "Model::A", "Null" { } }
Connections: { C: "OO", 1, 0 }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let limits = scene_draft::Limits::default();
    let draft = scene_draft::build(&doc, &limits).unwrap();
    assert_eq!(draft.index.objects.len(), 1);
    assert_eq!(draft.meshes.meshes.len(), 0);
    assert!(draft.retained_bytes > 0);
    let mut low = 0;
    let mut high = limits.max_allocation;
    while low < high {
        let mid = low + (high - low) / 2;
        let candidate = scene_draft::Limits {
            max_allocation: mid,
            ..limits.clone()
        };
        if scene_draft::build(&doc, &candidate).is_ok() {
            high = mid
        } else {
            low = mid + 1
        }
    }
    assert!(low > 0);
    for (amount, ok) in [(low - 1, false), (low, true), (low + 1, true)] {
        let candidate = scene_draft::Limits {
            max_allocation: amount,
            ..limits.clone()
        };
        assert_eq!(scene_draft::build(&doc, &candidate).is_ok(), ok);
    }
    assert!(matches!(
        scene_draft::build(
            &doc,
            &scene_draft::Limits {
                max_allocation: 0,
                ..limits
            }
        ),
        Err(scene::Error::LimitExceeded("allocation"))
    ));
}

#[test]
fn static_draft_charges_surface_allocation() {
    let input = br#"; FBX 7.4.0
Objects: { Video: 1, "Video::A", "Clip" { Content: , "AQIDBA==" } }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let limits = scene_draft::Limits::default();
    let surface_limits = fbx_rs::scene_surface::Limits::default();
    let core = scene_draft::build(&doc, &limits).unwrap();
    let complete = scene_draft::build_static(
        &doc,
        fbx_rs::scene_surface::Encoding::Ascii,
        &limits,
        &surface_limits,
    )
    .unwrap();
    assert!(complete.retained_bytes > core.retained_bytes);
    let limited = scene_draft::Limits {
        max_allocation: core.retained_bytes,
        ..limits
    };
    assert!(scene_draft::build_static(
        &doc,
        fbx_rs::scene_surface::Encoding::Ascii,
        &limited,
        &surface_limits
    )
    .is_err());
}
