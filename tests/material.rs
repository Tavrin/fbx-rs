use fbx_rs::{load, LoadOptions, MaterialMap};

const PBR: &[u8] = include_bytes!("fixtures/pbr-materials.fbx");

#[test]
fn class_id_pbr_maps_features_and_texture_switches() {
    let scene = load(PBR, &LoadOptions::default()).unwrap();
    assert!(scene.warnings().is_empty(), "{:?}", scene.warnings());
    assert_eq!(scene.materials().len(), 10);
    for (i, m) in scene.materials().iter().enumerate() {
        assert!(m.feature("pbr").unwrap().enabled);
        assert!(!m.feature("pbr").unwrap().is_explicit);
        let base = m.pbr_map("base_color").unwrap();
        assert!(base.has_value);
        assert_eq!(base.value[..3], [0.2, 0.3, 0.4]);
        assert_eq!(base.value[3], if i % 2 == 0 { 1.0 } else { 0.7 });
        assert_eq!(base.texture, Some(1));
    }
    for (i, m) in scene.materials()[..2].iter().enumerate() {
        let enabled = i == 1;
        assert_eq!(m.pbr_map("base_color").unwrap().texture_enabled, enabled);
        assert_eq!(
            m.pbr_map("subsurface_radius").unwrap().value[..3],
            [0.21; 3]
        );
        for name in [
            "specular",
            "diffuse_roughness",
            "thin_walled",
            "roughness_as_glossiness",
        ] {
            let f = m.feature(name).unwrap();
            assert!(f.is_explicit);
            assert_eq!(f.enabled, enabled);
        }
        let f = m.feature("transmission_roughness").unwrap();
        assert!(f.is_explicit);
        assert_eq!(f.enabled, !enabled);
        let rough = m.pbr_map("roughness").unwrap();
        let gloss = m.pbr_map("glossiness").unwrap();
        assert_eq!(rough.has_value, !enabled);
        assert_eq!(gloss.has_value, enabled);
    }
    // OpenPBR's emission weight is multiplied by luminance, rather than replaced.
    let open = &scene.materials()[8];
    assert_eq!(
        open.pbr_map("emission_factor").unwrap().value[0],
        0.30 * 0.31
    );
}

#[test]
fn material_map_remains_constructible_and_exhaustive_in_0_1() {
    let map = MaterialMap {
        value: [0.0; 4],
        value_int: 0,
        value_components: 0,
        texture: None,
        has_value: false,
        texture_enabled: false,
        feature_disabled: false,
    };
    let MaterialMap {
        value,
        value_int,
        value_components,
        texture,
        has_value,
        texture_enabled,
        feature_disabled,
    } = map;
    assert_eq!(value, [0.0; 4]);
    assert_eq!((value_int, value_components, texture), (0, 0, None));
    assert!(!has_value && !texture_enabled && !feature_disabled);
}

#[test]
fn physical_defaults_and_spec_gloss_feature_range() {
    let scene = load(
        br#"; FBX 7.4.0
Objects: {
 Material: 1, "Material::Physical", "" {
  ShadingModel: "unknown"
  Properties70: {
   P: "3dsMax|ClassIDa", "int", "", "", 1030429932
   P: "3dsMax|ClassIDb", "int", "", "", 3735928833
  }
 }
}
Connections: { }
"#,
        &LoadOptions::default(),
    )
    .unwrap();
    let m = &scene.materials()[0];
    for name in [
        "pbr",
        "metalness",
        "diffuse",
        "coat",
        "sheen",
        "transmission",
        "opacity",
        "ior",
    ] {
        let f = m.feature(name).unwrap();
        assert!(f.enabled && !f.is_explicit, "{name}");
    }
    assert!(!m.feature("specular").unwrap().enabled);
    for (value, expected) in [("2", false), ("0.75", true)] {
        let input = std::str::from_utf8(PBR).unwrap().replace(
            "3dsMax|main|useGlossiness\", \"int\", \"\", \"\", 1",
            &format!("3dsMax|main|useGlossiness\", \"Number\", \"\", \"\", {value}"),
        );
        let scene = load(input.as_bytes(), &LoadOptions::default()).unwrap();
        for i in [3, 5] {
            let f = scene.materials()[i]
                .feature("roughness_as_glossiness")
                .unwrap();
            assert!(f.is_explicit);
            assert_eq!(f.enabled, expected);
        }
    }
}
