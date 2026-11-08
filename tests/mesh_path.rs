#![cfg(feature = "meshoptimizer")]
use fbx_rs::meshoptimizer_rs::{
    simplify_with_attributes, SimplifyOptions, SimplifySettings, Workspace,
};
use fbx_rs::{load, LoadOptions, MeshoptOptions};

/// Two quads sharing an edge, with a UV seam along it and one material per
/// quad (the first quad uses slot 1).
const TWO_QUADS: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Geometry: 1, "Geometry::Quads", "Mesh" {
  Vertices: *18 { a: 0,0,0, 1,0,0, 1,1,0, 0,1,0, 2,0,0, 2,1,0 }
  PolygonVertexIndex: *8 { a: 0,1,2,-4, 1,4,5,-3 }
  LayerElementUV: 0 {
   Name: "UV0"
   MappingInformationType: "ByPolygonVertex"
   ReferenceInformationType: "Direct"
   UV: *16 { a: 0,0, 1,0, 1,1, 0,1, 0,0, 1,0, 1,1, 0,1 }
  }
  LayerElementMaterial: 0 {
   MappingInformationType: "ByPolygon"
   ReferenceInformationType: "IndexToDirect"
   Materials: *2 { a: 1,0 }
  }
  Layer: 0 {
   LayerElement: { Type: "LayerElementUV"
    TypedIndex: 0 }
   LayerElement: { Type: "LayerElementMaterial"
    TypedIndex: 0 }
  }
 }
 Model: 2, "Model::Quads", "Mesh" { }
 Material: 3, "Material::Left", "" { ShadingModel: "phong" }
 Material: 4, "Material::Right", "" { ShadingModel: "phong" }
}
Connections: {
 C: "OO",1,2
 C: "OO",2,0
 C: "OO",4,2
 C: "OO",3,2
}
"#;

#[test]
fn seams_split_and_slots_group() {
    let scene = load(TWO_QUADS.as_bytes(), &LoadOptions::default()).unwrap();
    let mesh = &scene.meshes()[0];
    let input = mesh.to_meshopt(&MeshoptOptions::default()).unwrap();
    // Six positions; the two on the shared edge are split by the UV seam.
    assert_eq!(input.vertex_count(), 8);
    assert_eq!(input.indices().len(), 12);
    assert_eq!(input.degenerate_triangles(), 0);
    assert_eq!(input.uvs().map(<[_]>::len), Some(8));
    assert_eq!(input.normals().map(<[_]>::len), Some(8));
    assert!(input.tangents().is_none());

    let name = |part: &fbx_rs::MeshoptPart| part.material.map(|m| scene.materials()[m].name());
    let parts = input.parts();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].material_slot, 0);
    assert_eq!(name(&parts[0]), Some("Right"));
    assert_eq!(parts[1].material_slot, 1);
    assert_eq!(name(&parts[1]), Some("Left"));
    // Slot 0 holds the second face: its corners come first.
    assert!(input.source_corners()[..4].iter().all(|&c| c >= 4));

    let attributes = input.attribute_stream().unwrap();
    assert_eq!(attributes.components(), 5);
    let mut workspace = Workspace::default();
    for part in parts {
        let lod = simplify_with_attributes(
            input.part_indices(part),
            input.position_view(),
            attributes.view().unwrap(),
            &[0.5, 0.5, 0.5, 1.0, 1.0],
            None,
            SimplifySettings {
                target_index_count: 3,
                target_error: 1.0,
                options: SimplifyOptions::EMPTY,
            },
            &mut workspace,
        )
        .unwrap();
        assert!(lod.indices.len() <= part.index_count);
    }
}

/// Every fixture mesh converts; indices stay in range, the parts cover the
/// index buffer, and the dropped triangles match the import report.
#[test]
fn fixtures_convert() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut meshes = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let Ok(scene) = load(&std::fs::read(&path).unwrap(), &LoadOptions::default()) else {
            continue;
        };
        let report = scene.import_report().unwrap();
        for (mesh, info) in scene.meshes().iter().zip(&report.meshes) {
            let input = mesh.to_meshopt(&MeshoptOptions::default()).unwrap();
            let n = input.vertex_count();
            assert!(
                input.indices().iter().all(|&i| (i as usize) < n),
                "{path:?}"
            );
            let covered: usize = input.parts().iter().map(|p| p.index_count).sum();
            assert_eq!(covered, input.indices().len(), "{path:?}");
            assert_eq!(
                input.degenerate_triangles(),
                info.degenerate_triangles,
                "{path:?}"
            );
            assert_eq!(
                input.indices().len() / 3 + input.degenerate_triangles(),
                info.triangles,
                "{path:?}"
            );
            meshes += 1;
        }
    }
    assert!(meshes > 0);
}
