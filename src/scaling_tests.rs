//! Deterministic growth guards: count the converter's charged work, never time.
use crate::canon_scene::{project_with, DumpLimits};
use crate::conversion_budget::COUNTERS;
use std::fmt::Write;

fn input(kind: usize, n: usize) -> String {
    let mut objects = String::new();
    let mut connections = String::new();
    if kind == 0 {
        objects.push_str("Deformer: 900000, \"Deformer::Skin\", \"Skin\" { }\n");
    }
    if kind == 3 {
        objects.push_str("AnimationLayer: 900001, \"AnimLayer::Shared\", \"\" { }\n");
        connections.push_str("C: \"OO\",900001,900000\n");
    }
    if kind == 4 {
        // N keys and N duplicate links: cached helper ranges must not decode
        // the key array once per link, and DOM lookup must not scan all nodes.
        objects.push_str("AnimationCurveNode: 900002, \"AnimCurveNode::S\", \"\" { }\nAnimationCurve: 900003, \"AnimCurve::S\", \"\" {\n");
        let ticks = (0..n).map(|k| k.to_string()).collect::<Vec<_>>().join(",");
        let values = vec!["2"; n].join(",");
        writeln!(objects, "KeyTime: *{n} {{ a: {ticks} }}\nKeyValueFloat: *{n} {{ a: {values} }}\nKeyAttrFlags: *1 {{ a: 4 }}\nKeyAttrDataFloat: *4 {{ a: 0,0,0,0 }}\nKeyAttrRefCount: *1 {{ a: {n} }}\n }}").unwrap();
        for _ in 0..n {
            connections.push_str("C: \"OP\",900003,900002,\"d|X\"\n");
        }
        connections.push_str("C: \"OP\",900002,10,\"Lcl Scaling\"\n");
    }
    if kind == 1 {
        objects.push_str("Pose: 900000, \"Pose::Bind\", \"BindPose\" {\n");
        for i in (1..=n).rev() {
            writeln!(objects, "PoseNode: {{ Node: {i}\n Matrix: *16 {{ a: 1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1 }}\n }}").unwrap();
        }
        objects.push_str("}\n");
    }
    if kind == 2 || kind == 3 {
        objects.push_str("AnimationStack: 900000, \"AnimStack::Take\", \"\" { }\n");
    }
    for i in 1..=n {
        let id = i * 10;
        match kind {
            0 => {
                writeln!(objects, "Deformer: {id}, \"SubDeformer::Cluster\", \"Cluster\" {{\n Indexes: *0 {{ a: }}\n Weights: *0 {{ a: }}\n }}\nModel: {}, \"Model::Bone\", \"LimbNode\" {{ }}", id + 1).unwrap();
                // N distinct clusters and 4N edges into one skin.
                for _ in 0..4 {
                    writeln!(connections, "C: \"OO\",{id},900000").unwrap();
                }
                writeln!(connections, "C: \"OO\",{},{id}", id + 1).unwrap();
            }
            1 => {
                writeln!(objects, "Model: {i}, \"Model::Bone\", \"LimbNode\" {{ }}").unwrap();
                writeln!(connections, "C: \"OO\",{i},{}", i - 1).unwrap();
            }
            2 | 3 => {
                writeln!(objects, "AnimationCurveNode: {id}, \"AnimCurveNode::T\", \"\" {{ }}\n AnimationCurve: {}, \"AnimCurve::X\", \"\" {{\n KeyTime: *2 {{ a: 0,46186158000 }}\n KeyValueFloat: *2 {{ a: 0,10 }}\n KeyAttrFlags: *1 {{ a: 4 }}\n KeyAttrDataFloat: *4 {{ a: 0,0,0,0 }}\n KeyAttrRefCount: *1 {{ a: 2 }}\n }}", id + 1).unwrap();
                writeln!(connections, "C: \"OP\",{},{id},\"d|X\"", id + 1).unwrap();
                if kind == 3 {
                    for _ in 0..4 {
                        writeln!(connections, "C: \"OO\",{id},900001").unwrap();
                    }
                    writeln!(objects, "AnimationLayer: {}, \"AnimLayer::Layer\", \"\" {{ }}\n Model: {}, \"Model::Node\", \"Null\" {{ }}", id + 2, id + 3).unwrap();
                    for _ in 0..4 {
                        writeln!(
                            connections,
                            "C: \"OO\",{id},{}\n C: \"OO\",{},900000",
                            id + 2,
                            id + 2
                        )
                        .unwrap();
                    }
                    writeln!(connections, "C: \"OP\",{id},{},\"Lcl Translation\"", id + 3).unwrap();
                }
            }
            4 => {
                writeln!(objects, "Model: {id}, \"Model::Parent\", \"Null\" {{ Properties70: {{ P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\", 2,3,4 }}\n }}\n Model: {}, \"Model::Child\", \"Null\" {{ Properties70: {{ P: \"InheritType\", \"enum\", \"\", \"\", 0 }}\n }}", id + 1).unwrap();
                writeln!(connections, "C: \"OO\",{id},0\n C: \"OO\",{},{id}", id + 1).unwrap();
            }
            _ => unreachable!(),
        }
    }
    format!("; FBX 7.4.0\nFBXHeaderExtension: {{ FBXVersion: 7400 }}\nObjects: {{\n{objects}}}\nConnections: {{\n{connections}}}\n")
}

#[test]
fn graph_work_grows_with_input_and_index_sorting() {
    for (kind, name) in ["deformers", "poses", "curves", "layers", "helpers"]
        .iter()
        .enumerate()
    {
        let measure = |n| {
            COUNTERS.with(|c| *c.borrow_mut() = [0; 5]);
            let text = input(kind, n);
            let s = project_with(text.as_bytes(), &DumpLimits::default(), true).unwrap();
            match kind {
                0 => {
                    assert_eq!(s.deformers.skins.len(), 1);
                    assert_eq!(s.deformers.skins[0].1.clusters.len(), n);
                }
                1 => assert_eq!(s.deformers.poses[0].1.bone_poses.len(), n),
                2 => assert_eq!(s.animation.curves.len(), n),
                3 => {
                    assert_eq!(s.animation.layers.len(), n + 1);
                    // Layers follow file order, so find the large one.
                    let widest = s.animation.layers.iter().map(|l| l.values.len()).max();
                    assert_eq!(widest, Some(n));
                }
                4 => {
                    // Componentwise inheritance propagates helpers to both
                    // the nonuniform parent and its componentwise child.
                    assert_eq!(s.scale_helpers.len(), 2 * n);
                }
                _ => unreachable!(),
            }
            COUNTERS.with(|c| c.borrow()[kind])
        };
        let small = measure(32);
        let large = measure(256);
        eprintln!(
            "{name}: N=32 {small}, 8N=256 {large}, ratio={:.4}",
            large as f64 / small as f64
        );
        assert!(small > 0 && large > small);
        // 8 * log2(256)/log2(32), with a small fixed allowance for rounding.
        assert!(large <= small * 14, "{name}: {small} -> {large}");
    }
}
