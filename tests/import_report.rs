use fbx_rs::{load, Axis, CoordinateAxes, LoadOptions};

/// A Z-up centimeter file: a three-triangle fan sharing one edge, a
/// degenerate triangle and a line face; a geometric translation on the mesh
/// node; a non-uniformly scaled parent whose child ignores its scale; one
/// material with a file texture and a texture without any file.
const ASSET: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension: { FBXVersion: 7400 }
GlobalSettings: { Properties70: {
 P: "UpAxis", "int", "Integer", "",2
 P: "UpAxisSign", "int", "Integer", "",1
 P: "FrontAxis", "int", "Integer", "",1
 P: "FrontAxisSign", "int", "Integer", "",-1
 P: "CoordAxis", "int", "Integer", "",0
 P: "CoordAxisSign", "int", "Integer", "",1
 P: "UnitScaleFactor", "double", "Number", "",1
} }
Objects: {
 Geometry: 1, "Geometry::Fan", "Mesh" {
  Vertices: *21 { a: 0,0,0, 1,0,0, 0,1,0, 0,-1,0, 0,0,1, 2,2,2, 3,3,3 }
  PolygonVertexIndex: *14 { a: 0,1,-3, 0,1,-4, 0,1,-5, 5,5,-7, 2,-4 }
 }
 Model: 2, "Model::Fan", "Mesh" {
  Properties70: {
   P: "GeometricTranslation", "Vector3D", "Vector", "",1,0,0
  }
 }
 Model: 3, "Model::Parent", "Null" {
  Properties70: {
   P: "Lcl Scaling", "Lcl Scaling", "", "A",2,3,4
  }
 }
 Model: 4, "Model::Child", "Null" {
  Properties70: {
   P: "InheritType", "enum", "", "",2
  }
 }
 Material: 5, "Material::Mat", "" {
  ShadingModel: "phong"
 }
 Texture: 6, "Texture::Albedo", "" { FileName: "albedo.png" }
 Texture: 7, "Texture::Missing", "" { }
}
Connections: {
 C: "OO",1,2
 C: "OO",2,0
 C: "OO",5,2
 C: "OP",6,5,"DiffuseColor"
 C: "OP",7,5,"NormalMap"
 C: "OO",3,0
 C: "OO",4,3
}
"#;

#[test]
fn report_covers_conversion_bakes_topology_and_textures() {
    let scene = load(ASSET.as_bytes(), &LoadOptions::default()).unwrap();
    let report = scene.import_report().unwrap();

    let space = &report.space;
    assert_eq!(
        space.source_axes,
        Some(CoordinateAxes {
            right: Axis::PositiveX,
            up: Axis::PositiveZ,
            front: Axis::NegativeY,
        })
    );
    assert_eq!(space.target_axes, CoordinateAxes::RIGHT_HANDED_Y_UP);
    assert!(space.axes_converted());
    assert_eq!(space.source_unit_meters, 0.01);
    assert_eq!(space.target_unit_meters, 1.0);
    assert_eq!(space.geometry_scale, 0.01);
    assert!(space.unit_converted());
    assert_eq!(space.mirror_axis, None);
    assert_ne!(space.root_rotation, [0.0, 0.0, 0.0, 1.0]);

    let node = |name: &str| scene.nodes().iter().position(|n| n.name() == name);
    assert_eq!(report.scale_helpers.len(), 1);
    assert_eq!(Some(report.scale_helpers[0].node), node("Parent"));
    let helper = &scene.nodes()[report.scale_helpers[0].helper];
    assert_eq!(helper.parent(), node("Parent"));
    assert_eq!(report.baked_geometry_transforms, [node("Fan").unwrap()]);

    let mesh = &report.meshes[0];
    assert_eq!(mesh.mesh, 0);
    assert_eq!(mesh.faces, 5);
    assert_eq!(mesh.skipped_faces, 1);
    assert_eq!(mesh.triangles, 4);
    assert_eq!(mesh.degenerate_triangles, 1);
    // The shared edge is used three times; the six outer edges once each.
    assert_eq!(mesh.non_manifold_edges, 1);
    assert_eq!(mesh.boundary_edges, 6);
    assert!(mesh.generated_normals);
    assert!(mesh.missing_uvs);
    assert!(!mesh.reversed_winding);

    assert_eq!(report.textures.len(), 2);
    let albedo = &report.textures[0];
    assert_eq!(albedo.property, "DiffuseColor");
    assert_eq!(scene.textures()[albedo.texture].name(), "Albedo");
    assert!(albedo.has_filename && !albedo.embedded && !albedo.is_unresolved());
    let missing = &report.textures[1];
    assert_eq!(missing.property, "NormalMap");
    assert!(missing.is_unresolved());
}

/// The report succeeds on every fixture, with one entry per mesh.
#[test]
fn report_on_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut loaded = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let Ok(scene) = load(&std::fs::read(&path).unwrap(), &LoadOptions::default()) else {
            continue;
        };
        let report = scene.import_report().unwrap();
        assert_eq!(report.meshes.len(), scene.meshes().len(), "{path:?}");
        for (m, r) in scene.meshes().iter().zip(&report.meshes) {
            assert_eq!(r.triangles, m.triangles().len() / 3, "{path:?}");
            assert!(r.degenerate_triangles <= r.triangles, "{path:?}");
        }
        loaded += 1;
    }
    assert!(loaded > 0);
}

#[test]
fn cached_reports_preserve_helpers_and_space_conversions() {
    use fbx_rs::cache::{self, CacheKey};
    use fbx_rs::Limits;

    for asset in [
        ASSET.to_owned(),
        ASSET.replace(
            "P: \"CoordAxisSign\", \"int\", \"Integer\", \"\",1",
            "P: \"CoordAxisSign\", \"int\", \"Integer\", \"\",-1",
        ),
        ASSET.replace(
            "P: \"UnitScaleFactor\", \"double\", \"Number\", \"\",1",
            "P: \"UnitScaleFactor\", \"double\", \"Number\", \"\",100",
        ),
    ] {
        let options = LoadOptions::default();
        let scene = load(asset.as_bytes(), &options).unwrap();
        let report = scene.import_report().unwrap();
        assert!(!report.scale_helpers.is_empty());
        assert!(!report.baked_geometry_transforms.is_empty());
        let limits = Limits::default();
        let key = CacheKey::new(b"report-conversions", &options, &limits).unwrap();
        let mut bytes = Vec::new();
        scene.write_cache(&key, &mut bytes).unwrap();
        let cached = cache::read(&bytes, &key, &limits).unwrap();
        assert_eq!(cached.import_report().unwrap(), report);
        let mut rewritten = Vec::new();
        cached.write_cache(&key, &mut rewritten).unwrap();
        assert_eq!(rewritten, bytes);
    }
}
