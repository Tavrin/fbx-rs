use fbx_rs::{load, load_with_limits, Interpolation, Limits, LoadOptions};

fn fixture(times: &str, values: &str, flags: &str, attrs: &str, refs: &str) -> String {
    format!(
        r#"; FBX 7.4.0 project file
FBXHeaderExtension: {{ FBXVersion: 7400 }}
Objects: {{
 Model: 1, "Model::Animated", "Null" {{
  Properties70: {{ P: "Lcl Translation", "Lcl Translation", "", "A", 0,0,0 }}
 }}
 AnimationStack: 2, "AnimStack::Take", "" {{
  Properties70: {{
   P: "LocalStart", "KTime", "Time", "", 0
   P: "LocalStop", "KTime", "Time", "", 46186158000
  }}
 }}
 AnimationLayer: 3, "AnimLayer::Base", "" {{ }}
 AnimationCurveNode: 4, "AnimCurveNode::T", "" {{
  Properties70: {{
   P: "d|X", "Number", "", "A", 0
   P: "d|Y", "Number", "", "A", 2
   P: "d|Z", "Number", "", "A", 3
  }}
 }}
 AnimationCurve: 5, "AnimCurve::X", "" {{
  KeyTime: *2 {{ a: {times} }}
  KeyValueFloat: *2 {{ a: {values} }}
  KeyAttrFlags: *1 {{ a: {flags} }}
  KeyAttrDataFloat: *4 {{ a: {attrs} }}
  KeyAttrRefCount: *1 {{ a: {refs} }}
 }}
}}
Connections: {{
 C: "OO",1,0
 C: "OO",3,2
 C: "OO",4,3
 C: "OP",4,1,"Lcl Translation"
 C: "OP",5,4,"d|X"
}}
"#
    )
}

#[test]
fn linear_property_and_scene_evaluation() {
    let bytes = fixture("0,46186158000", "0,10", "4", "0,0,0,0", "2");
    let s = load(bytes.as_bytes(), &LoadOptions::default()).unwrap();
    assert!(s.unsupported().is_empty());
    assert_eq!(s.animation_stacks().len(), 1);
    assert_eq!(s.animation_layers()[0].bindings.len(), 1);
    assert_eq!(
        s.animation_curves()[0].keys[0].interpolation,
        Interpolation::Linear
    );
    let p = s
        .evaluate_property(Some(0), 1, b"Lcl Translation", 0.25)
        .unwrap();
    assert_eq!(p.components, [2.5, 2.0, 3.0, 0.0]);
    assert_ne!(p.flags & 0x2000, 0);
    let pose = s.evaluate(None, 0.25).unwrap();
    assert_eq!(pose.nodes.len(), s.nodes().len());
    // Default FBX centimetres are converted to metres.
    assert_eq!(
        pose.nodes[1].local_transform.translation,
        [0.025, 0.02, 0.03]
    );
    assert_eq!(
        s.evaluate_transform(Some(0), 1, 0.25).unwrap(),
        pose.nodes[1].local_transform
    );
    assert_eq!(
        s.evaluate_binding(None, &s.animation_layers()[0].bindings[0], 0.25)
            .unwrap(),
        p
    );
    assert_eq!(s.animation_curves()[0].evaluate(-1.0, -5.0).unwrap(), 0.0);
    assert_eq!(s.animation_curves()[0].evaluate(2.0, -5.0).unwrap(), 10.0);
    assert!(s.evaluate(Some(1), 0.0).is_err());
    assert!(s.evaluate(None, f64::NAN).is_err());
    assert!(s.animation_curves()[0]
        .evaluate(f64::INFINITY, 0.0)
        .is_err());
    assert!(s.evaluate_transform(None, usize::MAX, 0.0).is_err());
}

#[test]
fn constant_and_cubic_segments_preserve_endpoints() {
    for (flags, midpoint, kind) in [
        (2, 0.0, Interpolation::ConstantPrevious),
        (258, 10.0, Interpolation::ConstantNext),
        (1032, 5.0, Interpolation::Cubic),
    ] {
        let bytes = fixture("0,46186158000", "0,10", &flags.to_string(), "0,0,0,0", "2");
        let s = load(bytes.as_bytes(), &LoadOptions::default()).unwrap();
        let c = &s.animation_curves()[0];
        assert_eq!(c.keys[0].interpolation, kind);
        assert_eq!(c.evaluate(0.0, -1.0).unwrap(), 0.0);
        assert_eq!(c.evaluate(1.0, -1.0).unwrap(), 10.0);
        assert_eq!(c.evaluate(0.5, -1.0).unwrap(), midpoint);
    }
}

#[test]
fn malformed_curve_counts_and_times_are_errors() {
    for input in [
        fixture("46186158000,0", "0,10", "4", "0,0,0,0", "2"),
        fixture("0,NaN", "0,10", "4", "0,0,0,0", "2"),
        fixture("0,46186158000", "0,10", "4", "0,0,0,0", "2147483647"),
        fixture("0,46186158000", "0,10", "4", "0,0,0,0", "-1"),
        fixture("0,46186158000", "0", "4", "0,0,0,0", "2"),
    ] {
        assert!(load(input.as_bytes(), &LoadOptions::default()).is_err());
    }
    let input = fixture("0,46186158000", "0,10", "4", "0,0,0,0", "2");
    let mut limits = Limits {
        max_animation_keys: 1,
        ..Limits::default()
    };
    assert!(load_with_limits(input.as_bytes(), &LoadOptions::default(), &limits).is_err());
    limits.max_animation_keys = 2;
    assert!(load_with_limits(input.as_bytes(), &LoadOptions::default(), &limits).is_ok());
}

#[test]
fn duplicate_key_times_are_bounded_and_choose_last_key() {
    let input = fixture("0,0", "2,7", "4", "0,0,0,0", "2");
    let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
    let c = &s.animation_curves()[0];
    assert_eq!(c.evaluate(-1.0, 0.0).unwrap(), 2.0);
    assert_eq!(c.evaluate(0.0, 0.0).unwrap(), 7.0);
}

fn with_layer(input: String, mode: i32) -> String {
    let extra = format!(
        r#" AnimationLayer: 6, "AnimLayer::Extra", "" {{
 Properties70: {{
  P: "Weight", "Number", "", "A", 25
  P: "BlendMode", "enum", "", "", {mode}
 }}
 }}
 AnimationCurveNode: 7, "AnimCurveNode::Extra", "" {{
 Properties70: {{
  P: "d|X", "Number", "", "A", 4
  P: "d|Y", "Number", "", "A", 8
  P: "d|Z", "Number", "", "A", 16
 }}
 }}
}}
Connections:"#
    );
    input.replace("}\nConnections:", &extra).replace(" C: \"OP\",5,4,\"d|X\"", " C: \"OP\",5,4,\"d|X\"\n C: \"OO\",6,2\n C: \"OO\",7,6\n C: \"OP\",7,1,\"Lcl Translation\"")
}

#[test]
fn layer_composition_and_first_layer_weight() {
    for (mode, expected) in [
        (0, [3.5, 4.0, 7.0, 0.0]),
        (1, [4.0, 8.0, 16.0, 0.0]),
        (2, [2.875, 3.5, 6.25, 0.0]),
    ] {
        let input = with_layer(fixture("0,46186158000", "0,10", "4", "0,0,0,0", "2"), mode);
        let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
        assert_eq!(s.animation_stacks()[0].layers.len(), 2);
        assert_eq!(
            s.evaluate_property(None, 1, b"Lcl Translation", 0.25)
                .unwrap()
                .components,
            expected
        );
    }
}

#[test]
fn extrapolation_modes_and_repeat_limit() {
    for (mode, before, after) in [
        ("C", 0.0, 10.0),
        ("R", 7.5, 2.5),
        ("M", 2.5, 7.5),
        ("A", -2.5, 12.5),
        ("K", -2.5, 12.5),
    ] {
        let input=fixture("0,46186158000","0,10","4","0,0,0,0","2").replace("  KeyTime:",&format!("  Pre-Extrapolation: {{ Type: {mode}\n Repetition: -1\n }}\n  Post-Extrapolation: {{ Type: {mode}\n Repetition: -1\n }}\n  KeyTime:"));
        let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
        let c = &s.animation_curves()[0];
        assert!(
            (c.evaluate(-0.25, 0.0).unwrap() - before).abs() < 1e-6,
            "{mode}"
        );
        assert!(
            (c.evaluate(1.25, 0.0).unwrap() - after).abs() < 1e-6,
            "{mode}"
        );
    }
}

#[test]
fn finite_repeat_count_holds_the_repeated_endpoint() {
    let input = fixture("0,46186158000", "0,10", "4", "0,0,0,0", "2").replace(
        "  KeyTime:",
        "  Post-Extrapolation: { Type: R\n Repetition: 1\n }\n  KeyTime:",
    );
    let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(s.animation_curves()[0].evaluate(3.5, 0.0).unwrap(), 10.0);
}

#[test]
fn animated_integer_view_uses_reference_boundary_conversion() {
    let input = fixture(
        "0,46186158000",
        "9.223372036854776e18,9.223372036854776e18",
        "4",
        "0,0,0,0",
        "2",
    );
    let s = load(input.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(
        s.evaluate_property(None, 1, b"Lcl Translation", 0.5)
            .unwrap()
            .value_int,
        i64::MIN
    );
}
