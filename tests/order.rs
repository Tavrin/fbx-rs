use fbx_rs::{load, BakeOptions, LoadOptions};

/// Object IDs are deliberately not ascending in file order.
const UNSORTED_IDS: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Model: 500, "Model::A", "Null" { }
 Model: 100, "Model::B", "Null" { }
 Model: 50, "Model::C", "Null" { }
 Material: 30, "Material::Gamma", "" { }
 Material: 10, "Material::Alpha", "" { }
 Material: 20, "Material::Beta", "" { }
 AnimationStack: 9, "AnimStack::Spin", "" { }
 AnimationStack: 3, "AnimStack::Base", "" { }
}
Connections: {
 C: "OO",500,0
 C: "OO",100,0
 C: "OO",50,500
}
"#;

#[test]
fn lists_follow_file_order_and_nodes_follow_depth_then_file_order() {
    let scene = load(UNSORTED_IDS.as_bytes(), &LoadOptions::default()).unwrap();
    let names = |it: Vec<&str>| it.join(",");
    // Parents precede children; siblings keep the order of the file, not of
    // their object IDs.
    assert_eq!(
        names(scene.nodes().iter().map(|n| n.name()).collect()),
        ",A,B,C"
    );
    assert_eq!(
        names(scene.materials().iter().map(|m| m.name()).collect()),
        "Gamma,Alpha,Beta"
    );
    assert_eq!(
        names(
            scene
                .animation_stacks()
                .iter()
                .map(|s| s.name.as_str())
                .collect()
        ),
        "Spin,Base"
    );
    assert_eq!(scene.root().children(), &[1, 2]);
}

/// A node with no parent connection hangs from the root in ufbx, so its
/// children are one level deeper than it (found by the `load` fuzz target).
const ORPHAN: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Model: 1, "Model::C", "Null" { }
 Model: 2, "Model::A", "Null" { }
 Model: 3, "Model::B", "Null" { }
 Model: 4, "Model::D", "Null" { }
}
Connections: {
 C: "OO",1,0
 C: "OO",3,2
 C: "OO",4,1
}
"#;

#[test]
fn a_node_without_a_parent_connection_is_a_child_of_the_root() {
    let scene = load(ORPHAN.as_bytes(), &LoadOptions::default()).unwrap();
    let names: Vec<&str> = scene.nodes().iter().map(|n| n.name()).collect();
    // ufbx: depth 1 is C and the orphan A, depth 2 is D (under C) then B (under A).
    assert_eq!(names, ["", "C", "A", "D", "B"]);
    assert_eq!(scene.root().children(), &[1, 2]);
}

/// A scale helper bakes its siblings: `J3` has no animation of its own and
/// inherits no scale, so it is only baked because the helper of its animated
/// parent is.
const SCALE_HELPER: &str = r#"; FBX 7.7.0 project file
FBXHeaderExtension: { FBXVersion: 7700 }
Objects: {
 Model: 12, "Model::J2", "LimbNode" {
  Properties70: {
   P: "InheritType", "enum", "", "", 2
   P: "Lcl Scaling", "Lcl Scaling", "", "A+", 0.01, 0.01, 0.01
  }
 }
 Model: 13, "Model::J3", "LimbNode" {
  Properties70: {
   P: "InheritType", "enum", "", "", 2
   P: "Lcl Translation", "Lcl Translation", "", "A", 2, 0, 0
   P: "Lcl Scaling", "Lcl Scaling", "", "A", 0.015, 0.025, 0.035
  }
 }
 AnimationStack: 20, "AnimStack::Take", "" {
  Properties70: {
   P: "LocalStart", "KTime", "Time", "", 0
   P: "LocalStop", "KTime", "Time", "", 46186158000
  }
 }
 AnimationLayer: 21, "AnimLayer::Base", "" { }
 AnimationCurveNode: 22, "AnimCurveNode::S", "" {
  Properties70: {
   P: "d|X", "Number", "", "A", 0.01
   P: "d|Y", "Number", "", "A", 0.01
   P: "d|Z", "Number", "", "A", 0.01
  }
 }
 AnimationCurve: 23, "AnimCurve::", "" {
  KeyTime: *2 { a: 0,46186158000 }
  KeyValueFloat: *2 { a: 0.01,0.003 }
  KeyAttrFlags: *1 { a: 24840 }
  KeyAttrDataFloat: *4 { a: 0,0,218434821,0 }
  KeyAttrRefCount: *1 { a: 2 }
 }
}
Connections: {
 C: "OO",12,0
 C: "OO",13,12
 C: "OO",20,0
 C: "OO",21,20
 C: "OO",22,21
 C: "OP",22,12,"Lcl Scaling"
 C: "OP",23,22,"d|X"
 C: "OP",23,22,"d|Y"
 C: "OP",23,22,"d|Z"
}
"#;

#[test]
fn baking_a_scale_helper_also_bakes_its_unanimated_siblings() {
    let scene = load(SCALE_HELPER.as_bytes(), &LoadOptions::default()).unwrap();
    let mut options = BakeOptions::default();
    options.key_reduction_enabled = false;
    let baked = scene.bake_animation(None, &options).unwrap();
    let node = |name: &str| scene.nodes().iter().position(|n| n.name() == name).unwrap();
    let find = |name: &str| baked.nodes.iter().find(|b| b.node == node(name));
    let j3 = find("J3").expect("J3 is baked with its scale helper");
    // Its translation is resampled at the helper's baked scale keys (31
    // keys); rotation and scale only have the stack bounds.
    assert_eq!(j3.translation_keys.len(), 31);
    assert_eq!(j3.rotation_keys.len(), 2);
    assert_eq!(j3.scale_keys.len(), 2);
    // J2's scale helper and J3 are the only baked nodes.
    assert_eq!(baked.nodes.len(), 2);
    assert!(baked.nodes.windows(2).all(|w| w[0].node < w[1].node));
}

#[test]
fn canonical_count_includes_opaque_and_generated_elements() {
    let scene = load(UNSORTED_IDS.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(scene.element_count(), 9); // root + eight authored objects
    let input = br#"; FBX 7.4.0 project file
Objects: {
 Model: 1, "Model::N", "Null" { }
 Video: 2, "Video::V", "Clip" { }
 ObjectMetaData: 3, "ObjectMetaData::M", "" { }
 PrivateObject: 4, "PrivateObject::O", "Unknown" { }
}
Connections: { C: "OO",1,0 }
"#;
    let scene = load(input, &LoadOptions::default()).unwrap();
    assert_eq!(scene.nodes().len(), 2);
    assert_eq!(scene.element_count(), 5);
    let scene = load(SCALE_HELPER.as_bytes(), &LoadOptions::default()).unwrap();
    // Six authored objects, the root and one generated scale helper.
    assert_eq!(scene.element_count(), 8);
}

#[cfg(feature = "parity")]
#[test]
fn canonical_count_matches_projection_including_legacy_synthesis() {
    use fbx_rs::canon_scene::{project, DumpLimits};
    for input in [
        UNSORTED_IDS.as_bytes(),
        SCALE_HELPER.as_bytes(),
        include_bytes!("fixtures/legacy-deform.fbx"),
        include_bytes!("fixtures/legacy-take.fbx"),
    ] {
        let canon = project(input, &DumpLimits::default()).unwrap();
        let public = load(input, &LoadOptions::default()).unwrap();
        assert_eq!(public.element_count(), canon.elements.len());
    }
}
