use fbx_rs::{load, LoadOptions};

#[test]
fn standard_surface_semantic_bindings() {
    for (api, rough) in [
        ("ARNOLD_SHADER_ID", "specularRoughness"),
        ("OSL", "specular_roughness"),
    ] {
        // Both Entry orientations, duplicate semantics, reference-only texture
        // properties, and identity fallback are independently authored here.
        let input = format!(
            r#"; FBX 7.4.0 project file
FBXHeaderExtension: {{ FBXVersion: 7400 }}
Objects: {{
 Material: 1, "Material::Surface", "" {{
  ShadingModel: "unknown"
  Properties70: {{
   P: "weight", "double", "Number", "",0.75
   P: "metal_tex", "Reference", "", ""
   P: "rough", "double", "Number", "",0.25
   P: "rough_tex", "Reference", "", ""
   P: "base", "double", "Number", "",0.5
  }}
 }}
 Implementation: 2, "Implementation::Surface", "" {{
  Properties70: {{ P: "RenderAPI", "KString", "", "", "{api}" }}
 }}
 BindingTable: 3, "BindingTable::Surface", "" {{
  Entry: "weight", "FbxPropertyEntry", "metalness", "FbxSemanticEntry"
  Entry: "metalness", "FbxSemanticEntry", "metal_tex", "FbxPropertyEntry"
  Entry: "rough", "FbxPropertyEntry", "{rough}", "FbxSemanticEntry"
  Entry: "rough_tex", "FbxPropertyEntry", "{rough}", "FbxSemanticEntry"
 }}
 Texture: 4, "Texture::Metal", "" {{ FileName: "metal.png" }}
 Texture: 5, "Texture::Rough", "" {{ FileName: "rough.png" }}
}}
Connections: {{
 C: "OO",1,2
 C: "OO",3,2
 C: "OP",4,1,"metal_tex"
 C: "OP",5,1,"rough_tex"
}}
"#
        );
        let scene = load(input.as_bytes(), &LoadOptions::default()).unwrap();
        let mat = &scene.materials()[0];
        let metal = mat.pbr_map("metalness").unwrap();
        let roughness = mat.pbr_map("roughness").unwrap();
        assert_eq!(metal.value[0], 0.75);
        assert_eq!(roughness.value[0], 0.25);
        assert!(metal.texture.is_some() && metal.texture_enabled);
        assert!(roughness.texture.is_some() && roughness.texture_enabled);
        assert_ne!(metal.texture, roughness.texture);
        assert_eq!(mat.pbr_map("base_factor").unwrap().value[0], 0.5);
    }
}

#[test]
fn arnold_procedural_texture_retains_input_file_links() {
    let input = br#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Texture: 1, "Texture::Bump", "" {
  Properties70: {
   P: "3dsMax|ai_bump2d Parameters/Connections", "Compound", "", ""
   P: "3dsMax|ai_bump2d Parameters/Connections|bump_map", "Reference", "", ""
  }
 }
 Texture: 2, "Texture::Image", "" { FileName: "bump.png" }
}
Connections: {
 C: "OP",2,1,"3dsMax|ai_bump2d Parameters/Connections|bump_map"
}
"#;
    let scene = load(input, &LoadOptions::default()).unwrap();
    assert_eq!(scene.textures()[0].file_textures(), &[1]);
}
