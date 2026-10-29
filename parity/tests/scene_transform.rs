use fbx_rs::{scene, scene_transform, syntax};

#[test]
fn reads_direct_transforms_and_settings() {
    let input = br#"; FBX 7.4.0
GlobalSettings:  {
 Properties70:  {
  P: "UpAxis", "int", "Integer", "", 2
  P: "UnitScaleFactor", "double", "Number", "", 100.0
 }
}
Objects:  {
 Model: 1, "Model::A", "Null" {
  Properties70:  {
   P: "Lcl Translation", "Lcl Translation", "", "A", 2.0, -0.0, 4.0
   P: "RotationPivot", "Vector3D", "Vector", "", 1, 2, 3
   P: "RotationOrder", "enum", "", "", 5
  }
 }
}
Connections: { C: "OO", 1, 0 }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let transforms = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    let transform = transforms.models[0].as_ref().unwrap();
    assert_eq!(transform.translation, [2.0, -0.0, 4.0]);
    assert!(transform.translation[1].is_sign_negative());
    assert_eq!(transform.rotation_pivot, [1.0, 2.0, 3.0]);
    assert_eq!(transform.rotation_order, 5);
    assert_eq!(transform.scaling, [1.0; 3]);
    assert_eq!(transforms.global_settings.up_axis, Some(2));
    assert_eq!(transforms.global_settings.unit_scale_factor, 100.0);
    assert_eq!(
        scene_transform::authored(&doc, &index, 0).unwrap_err(),
        scene::Error::LimitExceeded("allocation")
    );
}

#[test]
fn pivots_rotation_order_and_rotation_space() {
    let t = scene_transform::Transform {
        translation: [3.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 90.0],
        rotation_pivot: [1.0, 0.0, 0.0],
        scaling: [2.0, 1.0, 1.0],
        ..Default::default()
    };
    let m = t.local_matrix().unwrap();
    // Scaling moves the point to x=2; rotation around x=1 and translation follow.
    assert!((m[0][0] + m[0][3] - 4.0).abs() < 1e-12);
    assert!((m[1][0] + m[1][3] - 1.0).abs() < 1e-12);
    let bad = scene_transform::Transform {
        rotation_order: 7,
        ..t
    };
    assert_eq!(
        bad.local_matrix().unwrap_err(),
        scene::Error::InvalidTransform
    );
}

#[test]
fn model_template_is_overridden_by_direct_property() {
    let input = br#"; FBX 7.4.0
Definitions: {
 ObjectType: "Model" {
  PropertyTemplate: "FbxNode" {
   Properties70: {
    P: "Lcl Translation", "Lcl Translation", "", "A", 8, 9, 10
    P: "Lcl Scaling", "Lcl Scaling", "", "A", 2, 3, 4
   }
  }
 }
}
Objects: {
 Model: 1, "Model::A", "Null" {
  Properties70: { P: "Lcl Translation", "Lcl Translation", "", "A", 1, 2, 3 }
 }
}
Connections: { C: "OO", 1, 0 }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let result = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    let t = result.models[0].as_ref().unwrap();
    assert_eq!(t.translation, [1.0, 2.0, 3.0]);
    assert_eq!(t.scaling, [2.0, 3.0, 4.0]);
}

#[test]
fn inheritance_modes_and_limits() {
    let input = br#"; FBX 7.4.0
Objects: {
 Model: 1, "Model::A", "Null" { Properties70: { P: "Lcl Translation", "Lcl Translation", "", "A", 1, 0, 0 } }
 Model: 2, "Model::B", "Null" { Properties70: { P: "Lcl Translation", "Lcl Translation", "", "A", 0, 2, 0 } }
}
Connections: {
 C: "OO", 2, 1
 C: "OO", 1, 0
}
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let mut transforms = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    let world = scene_transform::world_matrices(&index, &transforms, usize::MAX).unwrap();
    let child = world[1].unwrap();
    assert_eq!([child[0][3], child[1][3], child[2][3]], [1.0, 2.0, 0.0]);
    transforms.models[1].as_mut().unwrap().inherit_type = 0;
    assert!(scene_transform::world_matrices(&index, &transforms, usize::MAX).is_ok());
    assert_eq!(
        scene_transform::world_matrices(&index, &transforms, 0).unwrap_err(),
        scene::Error::LimitExceeded("allocation")
    );
}

#[test]
fn settings_coordinate_basis_checks_axes_and_units() {
    let settings = scene_transform::GlobalSettings {
        coord_axis: Some(0),
        coord_axis_sign: Some(-1),
        up_axis: Some(2),
        up_axis_sign: Some(1),
        front_axis: Some(1),
        front_axis_sign: Some(1),
        unit_scale_factor: 100.0,
    };
    let basis = settings.source_to_y_up_meters().unwrap();
    assert_eq!(basis[0][0], -1.0);
    assert_eq!(basis[1][2], 1.0);
    assert_eq!(basis[2][1], 1.0);
    let invalid = scene_transform::GlobalSettings {
        front_axis: Some(2),
        ..settings
    };
    assert!(invalid.source_to_y_up_meters().is_none());
}

#[test]
fn allocation_thresholds_cover_transform_buffers() {
    let input = br#"; FBX 7.4.0
Objects: { Model: 1, "Model::A", "Null" {} }
Connections: { C: "OO", 1, 0 }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let transforms = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    for use_world in [false, true] {
        let ok = |limit| {
            if use_world {
                scene_transform::world_matrices(&index, &transforms, limit).is_ok()
            } else {
                scene_transform::authored(&doc, &index, limit).is_ok()
            }
        };
        let (mut low, mut high) = (0, 65536);
        while low < high {
            let mid = low + (high - low) / 2;
            if ok(mid) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        assert!(low > 0 && !ok(low - 1) && ok(low) && ok(low + 1));
    }
}

#[test]
fn pre_post_rotation_and_disabled_rotation_space() {
    let t = scene_transform::Transform {
        pre_rotation: [0.0, 0.0, 90.0],
        post_rotation: [0.0, 0.0, 90.0],
        rotation: [0.0, 0.0, 0.0],
        ..Default::default()
    };
    let m = t.local_matrix().unwrap();
    assert!((m[0][0] - 1.0).abs() < 1e-12);
    assert!(m[1][0].abs() < 1e-12);
    let disabled = scene_transform::Transform {
        rotation_active: 0,
        pre_rotation: [0.0, 0.0, 90.0],
        post_rotation: [0.0; 3],
        ..t
    };
    let m = disabled.local_matrix().unwrap();
    assert!((m[0][0] - 1.0).abs() < 1e-12);
    assert!(m[1][0].abs() < 1e-12);
}

#[test]
fn coordinate_conversion_scales_translation_once() {
    let settings = scene_transform::GlobalSettings {
        coord_axis: Some(0),
        up_axis: Some(2),
        front_axis: Some(1),
        unit_scale_factor: 1.0,
        ..Default::default()
    };
    let conversion = settings.y_up_meters_conversion().unwrap();
    assert!(conversion.reverses_handedness);
    assert_eq!(conversion.point([100.0, 200.0, 300.0]), [1.0, 3.0, 2.0]);
    let t = scene_transform::Transform {
        translation: [100.0, 200.0, 300.0],
        ..Default::default()
    };
    let matrix = conversion.matrix(t.local_matrix().unwrap());
    assert_eq!([matrix[0][3], matrix[1][3], matrix[2][3]], [1.0, 3.0, 2.0]);
    assert_eq!([matrix[0][0], matrix[1][1], matrix[2][2]], [1.0; 3]);
}

#[test]
fn inheritance_keeps_or_removes_nonuniform_parent_scale() {
    let input = br#"; FBX 7.4.0
Objects: {
 Model: 1, "Model::A", "Null" { }
 Model: 2, "Model::B", "Null" { }
}
Connections: {
 C: "OO", 2, 1
 C: "OO", 1, 0
}
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let mut transforms = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    transforms.models[0].as_mut().unwrap().scaling = [2.0, 3.0, 4.0];
    let child = transforms.models[1].as_mut().unwrap();
    child.translation = [1.0, 1.0, 1.0];
    child.inherit_type = 0;
    let matrix =
        scene_transform::world_matrices(&index, &transforms, usize::MAX).unwrap()[1].unwrap();
    assert_eq!([matrix[0][0], matrix[1][1], matrix[2][2]], [2.0, 3.0, 4.0]);
    assert_eq!([matrix[0][3], matrix[1][3], matrix[2][3]], [2.0, 3.0, 4.0]);
    transforms.models[1].as_mut().unwrap().inherit_type = 2;
    let matrix =
        scene_transform::world_matrices(&index, &transforms, usize::MAX).unwrap()[1].unwrap();
    assert_eq!([matrix[0][0], matrix[1][1], matrix[2][2]], [1.0, 1.0, 1.0]);
    assert_eq!([matrix[0][3], matrix[1][3], matrix[2][3]], [2.0, 3.0, 4.0]);
}

#[test]
fn plain_import_profile_projection_separates_basis_and_geometry_scale() {
    let settings = scene_transform::GlobalSettings {
        coord_axis: Some(0),
        up_axis: Some(2),
        front_axis: Some(1),
        unit_scale_factor: 1.0,
        ..Default::default()
    };
    let transform = scene_transform::Transform {
        translation: [100.0, 200.0, 300.0],
        ..Default::default()
    };
    let projected = settings
        .project_plain_world_y_up(transform.local_matrix().unwrap())
        .unwrap();
    assert_eq!(
        [projected[0][3], projected[1][3], projected[2][3]],
        [1.0, 3.0, 2.0]
    );
    assert_eq!(projected[0][0], 1.0);
    assert_eq!(projected[1][2], -1.0);
    assert_eq!(projected[2][1], 1.0);
}

#[test]
fn chained_ignore_parent_scale_uses_parent_inherited_scale_node() {
    let input = br#"; FBX 7.4.0
Objects: {
 Model: 1, "Model::A", "Null" { }
 Model: 2, "Model::B", "Null" { }
 Model: 3, "Model::C", "Null" { }
}

Connections: {
 C: "OO", 1, 0
 C: "OO", 2, 1
 C: "OO", 3, 2
}
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let mut transforms = scene_transform::authored(&doc, &index, usize::MAX).unwrap();
    transforms.models[0].as_mut().unwrap().scaling = [2.0, 3.0, 4.0];
    let b = transforms.models[1].as_mut().unwrap();
    b.inherit_type = 0;
    b.scaling = [5.0, 6.0, 7.0];
    let c = transforms.models[2].as_mut().unwrap();
    c.inherit_type = 2;
    c.translation = [1.0, 1.0, 1.0];
    let matrix =
        scene_transform::world_matrices(&index, &transforms, usize::MAX).unwrap()[2].unwrap();
    assert_eq!([matrix[0][0], matrix[1][1], matrix[2][2]], [2.0, 3.0, 4.0]);
    assert_eq!(
        [matrix[0][3], matrix[1][3], matrix[2][3]],
        [10.0, 18.0, 28.0]
    );
}

#[test]
fn leaf_attribute_projection_rejects_zero_scale() {
    let settings = scene_transform::GlobalSettings {
        coord_axis: Some(0),
        up_axis: Some(1),
        front_axis: Some(2),
        coord_axis_sign: Some(1),
        up_axis_sign: Some(1),
        front_axis_sign: Some(1),
        unit_scale_factor: 100.0,
    };
    let world = scene_transform::Transform::default()
        .local_matrix()
        .unwrap();
    assert!(settings
        .project_leaf_attribute_world_y_up(
            world,
            scene_transform::LeafAttribute::Camera,
            [1.0, 0.0, 1.0]
        )
        .is_none());
    let camera = settings
        .project_leaf_attribute_world_y_up(world, scene_transform::LeafAttribute::Camera, [1.0; 3])
        .unwrap();
    assert_eq!([camera[0][2], camera[1][1], camera[2][0]], [-1.0, 1.0, 1.0]);
    let scaled = scene_transform::Transform {
        scaling: [1.0, 2.0, 3.0],
        ..Default::default()
    }
    .local_matrix()
    .unwrap();
    let camera = settings
        .project_leaf_attribute_world_y_up(
            scaled,
            scene_transform::LeafAttribute::Camera,
            [1.0, 2.0, 3.0],
        )
        .unwrap();
    assert_eq!([camera[0][2], camera[1][1], camera[2][0]], [-3.0, 2.0, 1.0]);
}
