use fbx_rs::{load, load_with_limits, Error, Interpolation, Limit, Limits, LoadOptions};
const TAKE: &[u8] = include_bytes!("fixtures/legacy-take.fbx");

#[test]
fn named_connections_model_mesh_and_take() {
    let scene = load(TAKE, &LoadOptions::default()).unwrap();
    assert_eq!(scene.settings().version(), 6100);
    assert_eq!(
        scene.nodes().iter().map(|n| n.name()).collect::<Vec<_>>(),
        ["", "Parent", "Child"]
    );
    assert_eq!(scene.nodes()[2].parent(), Some(1));
    assert_eq!(scene.nodes()[1].source_id(), None);
    assert_eq!(scene.meshes()[0].source_id(), None);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].num_vertices(), 3);
    assert_eq!(scene.meshes()[0].triangles(), [0, 1, 2]);
    assert_eq!(scene.materials()[0].name(), "Red");
    assert_eq!(scene.animation_stacks().len(), 1);
    assert_eq!(scene.animation_layers().len(), 1);
    assert_eq!(scene.animation_values().len(), 1);
    assert_eq!(scene.animation_curves().len(), 1);
    assert_eq!(
        scene.animation_curves()[0].keys[0].interpolation,
        Interpolation::Linear
    );
    // Legacy objects have names rather than numeric source IDs. The binding
    // retains the target identity without exposing synthetic IDs as authored.
    let binding = &scene.animation_layers()[0].bindings[0];
    assert_eq!(binding.source_id, 0);
    assert_eq!(binding.property, b"Lcl Translation");
    assert_eq!(
        scene
            .evaluate_binding(Some(0), binding, 0.25)
            .unwrap()
            .components,
        [2.5, 2.0, 3.0, 0.0]
    );
    assert!(matches!(
        scene.evaluate_property(Some(0), 2, b"Lcl Translation", 0.25),
        Err(Error::InvalidData("animation: source ID not found"))
    ));
}

#[test]
fn legacy_generated_elements_and_keys_obey_limits() {
    for (limits, expected) in [
        (
            Limits {
                max_objects: 3,
                ..Limits::default()
            },
            Limit::Objects,
        ),
        (
            Limits {
                max_animation_keys: 1,
                ..Limits::default()
            },
            Limit::AnimationKeys,
        ),
        (
            Limits {
                max_animation_work: 1,
                ..Limits::default()
            },
            Limit::AnimationWork,
        ),
    ] {
        assert!(
            matches!(load_with_limits(TAKE,&LoadOptions::default(),&limits),Err(Error::LimitExceeded(l)) if l==expected)
        );
    }
}

#[test]
fn malformed_legacy_key_data_is_refused() {
    let text = std::str::from_utf8(TAKE).unwrap();
    for key in [
        "0,0,U,s",
        "0,0,Q",
        "0,0,L,46186158000",
        "0,0,U,a,q",
        "46186158000,0,L,0,1,L",
    ] {
        let input = text.replace("0,0,L,46186158000,10,L", key);
        assert!(
            load(input.as_bytes(), &LoadOptions::default()).is_err(),
            "{key}"
        );
    }
}

#[test]
fn big_endian_binary_mixed_keys_and_compressed_geometry() {
    for bytes in [
        include_bytes!("fixtures/legacy-le.fbx").as_slice(),
        include_bytes!("fixtures/legacy-be.fbx").as_slice(),
    ] {
        let scene = load(bytes, &LoadOptions::default()).unwrap();
        assert_eq!(scene.settings().version(), 6100);
        assert_eq!(scene.meshes()[0].triangles(), [0, 1, 2]);
        assert_eq!(
            scene.animation_curves()[0].evaluate(0.25, 0.0).unwrap(),
            2.5
        );
        assert_eq!(scene.animation_values()[0].default_value, [0.0, 2.0, 3.0]);
    }
}

#[test]
fn legacy_root_geometry_and_uv() {
    let scene = load(
        include_bytes!("fixtures/legacy-root.fbx"),
        &LoadOptions::default(),
    )
    .unwrap();
    assert_eq!(scene.settings().version(), 5800);
    assert_eq!(
        scene.nodes()[1].local_transform().translation,
        [0.02, 0.0, 0.0]
    );
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].triangles(), [0, 1, 2]);
    assert_eq!(scene.meshes()[0].uv_sets().len(), 1);
    assert_eq!(scene.animation_stacks().len(), 1);
}

#[test]
fn named_skin_pose_and_nested_shape() {
    let scene = load(
        include_bytes!("fixtures/legacy-deform.fbx"),
        &LoadOptions::default(),
    )
    .unwrap();
    assert_eq!(scene.skin_deformers().len(), 1);
    assert_eq!(scene.skin_clusters().len(), 1);
    assert_eq!(scene.bones().len(), 1);
    assert_eq!(scene.poses().len(), 1);
    assert_eq!(scene.blend_deformers().len(), 1);
    assert_eq!(scene.blend_channels().len(), 1);
    assert_eq!(scene.blend_shapes().len(), 1);
    assert_eq!(scene.meshes()[0].skin_deformers(), [0]);
}

#[test]
fn legacy_array_continuation_before_comma() {
    let text = std::str::from_utf8(TAKE).unwrap().replace(
        "Vertices: 0,0,0,1,0,0,0,1,0",
        "Vertices: 0,0,0\n  ,1,0,0\n  ,0,1,0",
    );
    assert_eq!(
        load(text.as_bytes(), &LoadOptions::default())
            .unwrap()
            .meshes()[0]
            .num_vertices(),
        3
    );
}

#[test]
fn legacy_binary_container_uses_declared_document_version() {
    let bytes = include_bytes!("fixtures/legacy-v5-binary.fbx");
    assert_eq!(
        load(bytes, &LoadOptions::default())
            .unwrap()
            .settings()
            .version(),
        5800
    );
    let mut older = bytes.to_vec();
    let offset = older
        .windows(8)
        .position(|w| w == 5800i64.to_le_bytes())
        .unwrap();
    older[offset..offset + 8].copy_from_slice(&5700i64.to_le_bytes());
    assert!(matches!(
        load(&older, &LoadOptions::default()),
        Err(Error::Parse(_))
    ));
}

#[test]
fn legacy_texture_layers_bind_material_maps() {
    let scene = load(
        include_bytes!("fixtures/legacy-texture.fbx"),
        &LoadOptions::default(),
    )
    .unwrap();
    assert_eq!(
        scene.materials()[0]
            .fbx_map("diffuse_color")
            .unwrap()
            .texture,
        Some(0)
    );
}

#[test]
fn legacy_unsupported_records_do_not_invent_authored_ids() {
    let text = std::str::from_utf8(TAKE).unwrap().replace(
        "Material: ",
        r#"Constraint: "Constraint::Unused", "Parent-Child" { }
 Material: "#,
    );
    let scene = load(text.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(scene.unsupported().len(), 1);
    assert_eq!(scene.unsupported()[0].source_id, 0);
}

#[test]
fn nonpositive_legacy_key_version_uses_format_default() {
    for version in [0, -1] {
        let text = std::str::from_utf8(TAKE)
            .unwrap()
            .replace("KeyVer: 4005", &format!("KeyVer: {version}"))
            .replace("0,0,L,46186158000,10,L", "0,0,C,n,46186158000,10,C,n");
        let scene = load(text.as_bytes(), &LoadOptions::default()).unwrap();
        assert_eq!(
            scene.animation_curves()[0].keys[0].interpolation,
            Interpolation::ConstantNext
        );
        assert_eq!(
            scene.animation_curves()[0].evaluate(0.25, 0.0).unwrap(),
            10.0
        );
    }
}

#[test]
fn legacy_keys_start_on_next_line_without_consuming_empty_nodes() {
    let text = std::str::from_utf8(TAKE).unwrap().replace(
        "Key: 0,0,L,46186158000,10,L",
        "Empty: \n ; comment before the next node\n Key: \n  0,0,L,46186158000,10,L",
    );
    let scene = load(text.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(scene.animation_curves().len(), 1);
    assert_eq!(scene.animation_curves()[0].keys.len(), 2);
    assert_eq!(
        scene.animation_curves()[0].evaluate(0.25, 0.0).unwrap(),
        2.5
    );
}

#[test]
fn legacy_texture_slots_follow_connections_instead_of_object_order() {
    let text = std::str::from_utf8(include_bytes!("fixtures/legacy-texture.fbx"))
        .unwrap()
        .replace(
            " Material:",
            r#" Texture: "Texture::Other", "TextureVideoClip" { FileName: "other.png" }
 Material:"#,
        )
        .replace(
            r#" Connect: "OO", "Texture::Color", "Model::Child""#,
            r#" Connect: "OO", "Texture::Other", "Model::Child"
 Connect: "OO", "Texture::Color", "Model::Child""#,
        );
    let scene = load(text.as_bytes(), &LoadOptions::default()).unwrap();
    assert_eq!(scene.textures()[0].name(), "Color");
    assert_eq!(scene.textures()[1].name(), "Other");
    assert_eq!(
        scene.materials()[0]
            .fbx_map("diffuse_color")
            .unwrap()
            .texture,
        Some(1)
    );
}
