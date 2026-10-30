//! Exact-bit bake stream for comparison with `reference/bake_probe.c`.
use fbx_rs::{BakeOptions, BakedKey};
fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".into();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn keys<const N: usize>(prefix: &str, keys: &[BakedKey<N>]) {
    for (i, k) in keys.iter().enumerate() {
        print!("{prefix} {i} {:016x} {}", k.time.to_bits(), k.flags);
        for v in &k.value {
            print!(" {:016x}", v.to_bits());
        }
        println!();
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = std::fs::read(&args[1])?;
    let scene = fbx_rs::load(&input, &fbx_rs::LoadOptions::default())?;
    let mut opts = BakeOptions::default();
    opts.trim_start_time = args.get(2).is_some_and(|s| s.contains("trim"));
    opts.key_reduction_enabled = !args.get(2).is_some_and(|s| s.contains("raw"));
    opts.key_reduction_rotation = args.get(2).is_some_and(|s| s.contains("rotation"));
    for (ix, stack) in scene.animation_stacks().iter().enumerate() {
        let bake = scene.bake_animation(Some(ix), &opts)?;
        println!(
            "S {} {:016x} {:016x} {:016x} {:016x} {:016x}",
            stack.source_id,
            bake.playback_time_begin.to_bits(),
            bake.playback_time_end.to_bits(),
            bake.playback_duration.to_bits(),
            bake.key_time_min.to_bits(),
            bake.key_time_max.to_bits()
        );
        for n in &bake.nodes {
            let node = &scene.nodes()[n.node];
            let id = format!(
                "{}:{}",
                node.source_id().unwrap_or(0),
                hex(node.name().as_bytes())
            );
            println!(
                "N {} {id} {} {} {} {} {} {}",
                stack.source_id,
                u8::from(n.constant_translation),
                u8::from(n.constant_rotation),
                u8::from(n.constant_scale),
                n.translation_keys.len(),
                n.rotation_keys.len(),
                n.scale_keys.len()
            );
            for (ch, channel) in [("t", &n.translation_keys), ("s", &n.scale_keys)] {
                keys(&format!("K {} {id} {ch}", stack.source_id), channel);
            }
            keys(&format!("K {} {id} r", stack.source_id), &n.rotation_keys);
        }
        for p in &bake.properties {
            let id = format!("{}:{}", p.source_id, hex(&p.name));
            println!(
                "P {} {id} {} {}",
                stack.source_id,
                u8::from(p.constant_value),
                p.keys.len()
            );
            keys(&format!("Q {} {id}", stack.source_id), &p.keys);
        }
    }
    Ok(())
}
