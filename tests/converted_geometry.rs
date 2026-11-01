use fbx_rs::{load, LoadOptions};

// Authored near-square: source diagonals differ by one ULP in one endpoint;
// centimetre conversion rounds the diagonals to a tie. ufbx.c:32416-32465
// selects split A on ties using loaded vertex_position, including winding.
#[test]
fn converted_quad_rounding_and_winding() {
    for front in [1, -1] {
        for unit in [1, 100] {
            let last = f64::from_bits(15.0f64.to_bits() - 1);
            let input = format!(
                r#"; FBX 7.4.0 project file
FBXHeaderExtension: {{ FBXVersion: 7400 }}
GlobalSettings: {{ Properties70: {{
 P: "UpAxis", "int", "Integer", "",1
 P: "UpAxisSign", "int", "Integer", "",1
 P: "FrontAxis", "int", "Integer", "",2
 P: "FrontAxisSign", "int", "Integer", "",{front}
 P: "CoordAxis", "int", "Integer", "",0
 P: "CoordAxisSign", "int", "Integer", "",1
 P: "UnitScaleFactor", "double", "Number", "",{unit}
}} }}
Objects: {{
 Geometry: 1, "Geometry::Square", "Mesh" {{
  Vertices: *12 {{ a: 1,14,0,2,14,0,2,15,0,1,{last},0 }}
  PolygonVertexIndex: *4 {{ a: 0,1,2,-4 }}
 }}
 Model: 2, "Model::Square", "Mesh" {{}}
}}
Connections: {{ C: "OO",1,2
 C: "OO",2,0
}}
"#
            );
            let scene = load(input.as_bytes(), &LoadOptions::default()).unwrap();
            let expected = if unit == 1 {
                [0, 1, 2, 2, 3, 0]
            } else {
                [1, 2, 3, 3, 0, 1]
            };
            assert_eq!(scene.meshes()[0].triangles(), expected);
        }
    }
}
