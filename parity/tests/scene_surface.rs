use fbx_rs::{scene, scene_surface, syntax};
use scene_surface::{Data, Encoding};
const INPUT: &[u8] = br#"; FBX 7.4.0
Definitions: {
 ObjectType: "Material" { PropertyTemplate: "FbxSurfacePhong" { Properties70: {
  P: "DiffuseFactor", "Number", "", "A", 0.5
  P: "DiffuseColor", "Color", "", "A", 0.1,0.2,0.3
 } } }
}
Objects: {
 Material: 1, "Material::M", "" {
  ShadingModel: "phong"
  Properties70: { P: "DiffuseFactor", "Number", "", "A", 0.8 }
 }
 Texture: 2, "Texture::T", "TextureVideoClip" {
  FileName: "image.png"
  RelativeFilename: "textures/image.png"
  Properties70: { P: "UVSet", "KString", "", "", "UV0" }
 }
 Video: 3, "Video::V", "Clip" { Content: , "AAEC/w==" }
 NodeAttribute: 4, "NodeAttribute::Camera", "Camera" {
  Properties70: { P: "FocalLength", "double", "Number", "", 35.0 }
 }
 NodeAttribute: 5, "NodeAttribute::Light", "Light" {
  Properties70: { P: "Intensity", "double", "Number", "", 150.0 }
 }
}
Connections: {
 C: "OP", 2, 1, "DiffuseColor"
 C: "OO", 3, 2
}
"#;
#[test]
fn material_texture_embedded_video_camera_light() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let surfaces =
        scene_surface::extract(&doc, &index, Encoding::Ascii, &Default::default()).unwrap();
    assert_eq!(surfaces.entries.len(), 5);
    let Data::Material(m) = &surfaces.entries[0].data else {
        panic!("material")
    };
    assert_eq!(m.shading_model, Some(&b"phong"[..]));
    assert_eq!(m.properties.diffuse_factor.unwrap().value, 0.8);
    assert!(m.properties.diffuse_factor.unwrap().authored);
    assert!(!m.properties.diffuse_color.unwrap().authored);
    let Data::Texture(t) = &surfaces.entries[1].data else {
        panic!("texture")
    };
    assert_eq!(t.uv_set, Some(&b"UV0"[..]));
    assert_eq!(
        surfaces.material_textures(0, &index).collect::<Vec<_>>(),
        vec![(1, &b"DiffuseColor"[..])]
    );
    assert_eq!(
        surfaces
            .texture_video(1, &index)
            .unwrap()
            .content
            .as_deref(),
        Some(&[0, 1, 2, 255][..])
    );
    let Data::Camera(c) = &surfaces.entries[3].data else {
        panic!("camera")
    };
    assert_eq!(c.focal_length.unwrap().value, 35.0);
    let Data::Light(l) = &surfaces.entries[4].data else {
        panic!("light")
    };
    assert_eq!(l.intensity.unwrap().value, 150.0);
    assert_eq!(surfaces.embedded_bytes, 4);
}
#[test]
fn limits_and_invalid_base64_are_explicit() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let limits = scene_surface::Limits {
        max_objects: 4,
        ..Default::default()
    };
    assert!(matches!(
        scene_surface::extract(&doc, &index, Encoding::Ascii, &limits),
        Err(scene::Error::LimitExceeded("surface objects"))
    ));
    let limits = scene_surface::Limits {
        max_embedded_bytes: 3,
        ..Default::default()
    };
    assert!(matches!(
        scene_surface::extract(&doc, &index, Encoding::Ascii, &limits),
        Err(scene::Error::LimitExceeded("embedded bytes"))
    ));
    let limits = scene_surface::Limits {
        max_allocation: 0,
        ..Default::default()
    };
    assert!(matches!(
        scene_surface::extract(&doc, &index, Encoding::Ascii, &limits),
        Err(scene::Error::LimitExceeded("allocation"))
    ));
    let input = String::from_utf8(INPUT.to_vec())
        .unwrap()
        .replace("AAEC/w==", "AA?C/w==");
    let doc = syntax::parse(input.as_bytes(), &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    assert!(matches!(
        scene_surface::extract(&doc, &index, Encoding::Ascii, &Default::default()),
        Err(scene::Error::InvalidEmbedded { .. })
    ));
}
#[test]
fn binary_embedded_content_is_borrowed() {
    use std::borrow::Cow;
    let doc = syntax::Document {
        version: 7400,
        nodes: vec![
            syntax::Node {
                name: b"Objects",
                parent: None,
                properties: 0..0,
            },
            syntax::Node {
                name: b"Video",
                parent: Some(0),
                properties: 0..3,
            },
            syntax::Node {
                name: b"Content",
                parent: Some(1),
                properties: 3..4,
            },
        ],
        properties: vec![
            syntax::Property::I64(1),
            syntax::Property::String(Cow::Borrowed(b"Video::V")),
            syntax::Property::String(Cow::Borrowed(b"Clip")),
            syntax::Property::Raw(&[0, 1, 2, 255]),
        ],
    };
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let surfaces =
        scene_surface::extract(&doc, &index, Encoding::Binary, &Default::default()).unwrap();
    let Data::Video(v) = &surfaces.entries[0].data else {
        panic!("video")
    };
    assert!(matches!(&v.content, Some(Cow::Borrowed(&[0, 1, 2, 255]))));
}

#[test]
fn subtype_prefix_and_matching_attribute_template() {
    let input = br#"; FBX 7.4.0
Definitions: {
 ObjectType: "NodeAttribute" { PropertyTemplate: "FbxLight" { Properties70: {
  P: "Intensity", "double", "Number", "", 250.0
 } } }
}
Objects: { NodeAttribute: 1, "NodeAttribute::L", "FbxLight" { } }
Connections: { }
"#;
    let doc = syntax::parse(input, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let surfaces =
        scene_surface::extract(&doc, &index, Encoding::Ascii, &Default::default()).unwrap();
    let Data::Light(light) = &surfaces.entries[0].data else {
        panic!("light")
    };
    assert_eq!(light.intensity.unwrap().value, 250.0);
    assert!(!light.intensity.unwrap().authored);
}

#[test]
fn surface_limit_boundaries() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    for (amount, ok) in [(4, false), (5, true), (6, true)] {
        let limits = scene_surface::Limits {
            max_objects: amount,
            ..Default::default()
        };
        assert_eq!(
            scene_surface::extract(&doc, &index, Encoding::Ascii, &limits).is_ok(),
            ok
        );
    }
    for (amount, ok) in [(3, false), (4, true), (5, true)] {
        let limits = scene_surface::Limits {
            max_embedded_bytes: amount,
            ..Default::default()
        };
        assert_eq!(
            scene_surface::extract(&doc, &index, Encoding::Ascii, &limits).is_ok(),
            ok
        );
    }
    let mut low = 0;
    let mut high = 1 << 20;
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = scene_surface::Limits {
            max_allocation: mid,
            ..Default::default()
        };
        if scene_surface::extract(&doc, &index, Encoding::Ascii, &limits).is_ok() {
            high = mid
        } else {
            low = mid + 1
        }
    }
    assert!(low > 0);
    for (amount, ok) in [(low - 1, false), (low, true), (low + 1, true)] {
        let limits = scene_surface::Limits {
            max_allocation: amount,
            ..Default::default()
        };
        assert_eq!(
            scene_surface::extract(&doc, &index, Encoding::Ascii, &limits).is_ok(),
            ok
        );
    }
}

#[test]
fn basic_phong_projection_preserves_value_and_texture_provenance() {
    let input=String::from_utf8(INPUT.to_vec()).unwrap().replace("P: \"DiffuseFactor\", \"Number\", \"\", \"A\", 0.8", "P: \"DiffuseFactor\", \"Number\", \"\", \"A\", 0.8\n   P: \"ShininessExponent\", \"Number\", \"\", \"A\", 25.0");
    let doc = syntax::parse(input.as_bytes(), &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let surfaces =
        scene_surface::extract(&doc, &index, Encoding::Ascii, &Default::default()).unwrap();
    let p = surfaces.basic_projection(0, &index, &doc).unwrap();
    assert_eq!(p.base_color.value, [0.1, 0.2, 0.3, 1.0]);
    assert!(p.base_color.has_value);
    assert_eq!(p.base_color.texture, Some(1));
    assert_eq!(p.base_factor.value, 0.8);
    assert!(p.base_factor.has_value);
    assert_eq!(p.roughness.value, 0.5);
    assert!(p.roughness.has_value);
    assert!(!p.specular_color.has_value);
}

#[test]
fn basic_light_and_camera_derived_values() {
    let doc = syntax::parse(INPUT, &syntax::Limits::default()).unwrap();
    let index = scene::index(&doc, &scene::Limits::default()).unwrap();
    let surfaces =
        scene_surface::extract(&doc, &index, Encoding::Ascii, &Default::default()).unwrap();
    let Data::Light(l) = &surfaces.entries[4].data else {
        panic!("light")
    };
    let l = l.basic_projection();
    assert_eq!(l.intensity, 1.5);
    assert_eq!(l.color, [1.0; 3]);
    assert!(l.cast_light);
    assert!(!l.cast_shadows);
    let Data::Camera(c) = &surfaces.entries[3].data else {
        panic!("camera")
    };
    let c = c.basic_projection(0.01);
    assert_eq!(c.focal_length_mm, 35.0);
    assert_eq!(c.near_plane, 0.0);
}
