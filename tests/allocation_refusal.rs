use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {static SWEEP:std::cell::Cell<Option<(usize,usize)>>=const{std::cell::Cell::new(None)};}
fn refused() -> bool {
    SWEEP
        .try_with(|s| match s.get() {
            Some((n, fail)) => {
                s.set(Some((n + 1, fail)));
                n >= fail
            }
            None => false,
        })
        .unwrap_or(false)
}
struct Refuse;
static SIZE: AtomicUsize = AtomicUsize::new(0);
static ALIGN: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Refuse {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if refused()
            || l.size() == SIZE.load(Ordering::Relaxed)
                && l.align() == ALIGN.load(Ordering::Relaxed)
        {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(l) }
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if refused()
            || n == SIZE.load(Ordering::Relaxed) && l.align() == ALIGN.load(Ordering::Relaxed)
        {
            std::ptr::null_mut()
        } else {
            unsafe { System.realloc(p, l, n) }
        }
    }
}
#[global_allocator]
static ALLOC: Refuse = Refuse;
#[test]
fn refused_public_node_array_returns_typed_error() {
    let _serial = SERIAL.lock().unwrap();
    let count = 20_000;
    let mut input = String::from(
        "; FBX 7.4.0 project file\nFBXHeaderExtension: { FBXVersion: 7400 }\nObjects: {\n",
    );
    for i in 1..=count {
        writeln!(input, "Model: {i}, \"Model::N{i}\", \"Null\" {{ }}").unwrap();
    }
    input.push_str("}\nConnections: {\n");
    for i in 1..=count {
        writeln!(input, "C: \"OO\",{i},0").unwrap();
    }
    input.push_str("}\n");
    let options = fbx_rs::LoadOptions::default();
    let limits = fbx_rs::Limits {
        max_input_bytes: 512 << 20,
        max_allocation: 768 << 20,
        ..Default::default()
    };
    let baseline = fbx_rs::load_with_limits(input.as_bytes(), &options, &limits).unwrap();
    assert_eq!(baseline.nodes().len(), count + 1);
    println!(
        "baseline_nodes={} node_size={} node_align={}",
        baseline.nodes().len(),
        size_of::<fbx_rs::Node>(),
        align_of::<fbx_rs::Node>()
    );
    drop(baseline);
    {
        SIZE.store((count + 1) * size_of::<fbx_rs::Node>(), Ordering::Relaxed);
        ALIGN.store(align_of::<fbx_rs::Node>(), Ordering::Relaxed);
    }
    let result = fbx_rs::load_with_limits(input.as_bytes(), &options, &limits);
    SIZE.store(0, Ordering::Relaxed);
    assert!(
        matches!(result, Err(fbx_rs::Error::Allocation)),
        "{result:?}"
    );
}

// A real allocator refuses the Nth allocation AND every allocation after it.
// Disarm before test diagnostics and Scene destruction. Any unreserved Vec,
// String, map node, sort scratch or error formatting on these paths aborts
// the test process instead of passing this assertion.
fn sweep<T>(name: &str, baseline_ok: bool, mut call: impl FnMut() -> Result<T, fbx_rs::Error>) {
    SWEEP.with(|s| s.set(Some((0, usize::MAX))));
    let baseline = call();
    let total = SWEEP.with(|s| s.replace(None).unwrap().0);
    assert_eq!(baseline.is_ok(), baseline_ok, "{name}: unexpected baseline");
    drop(baseline);
    assert!(total > 0);
    for n in 0..total {
        SWEEP.with(|s| s.set(Some((0, n))));
        let result = call();
        SWEEP.with(|s| s.set(None));
        assert!(
            matches!(result, Err(fbx_rs::Error::Allocation)),
            "{name}: refusal {n}/{total}"
        );
    }
    eprintln!("real allocator sweep {name}: {total} allocation points, all typed");
}
#[test]
fn full_load_evaluation_and_bake_refusals_are_typed() {
    let _serial = SERIAL.lock().unwrap();
    let input = include_bytes!("fixtures/allocation-rich.fbx");
    let binary = include_bytes!("fixtures/allocation-rich-binary.fbx");
    let options = fbx_rs::LoadOptions::default();
    let limits = fbx_rs::Limits::default();
    sweep("ASCII full load", true, || {
        fbx_rs::load_with_limits(input, &options, &limits)
    });
    sweep("compressed binary full load", true, || {
        fbx_rs::load_with_limits(binary, &options, &limits)
    });
    let scene = fbx_rs::load_with_limits(input, &options, &limits).unwrap();
    sweep("evaluate", true, || scene.evaluate(None, 0.5));
    sweep("property bytes", true, || {
        scene.evaluate_property(None, 2, b"Label", 0.5)
    });
    sweep("bake", true, || {
        scene.bake_animation(None, &fbx_rs::BakeOptions::default())
    });
}

#[test]
fn allocation_refusal_while_reporting_invalid_input_is_typed() {
    let _serial = SERIAL.lock().unwrap();
    let options = fbx_rs::LoadOptions::default();
    for input in [
        b"; FBX 7.4.0\nObjects: { Model: ".as_slice(),
        b"; FBX 5.7.0\nObjects: { }\n".as_slice(),
    ] {
        sweep("invalid input diagnostics", false, || {
            fbx_rs::load(input, &options)
        });
    }
}

#[test]
fn legacy_load_and_take_refusals_are_typed() {
    let _serial = SERIAL.lock().unwrap();
    let options = fbx_rs::LoadOptions::default();
    for (name, input) in [
        (
            "legacy named deformers",
            include_bytes!("fixtures/legacy-deform.fbx").as_slice(),
        ),
        (
            "legacy texture layers",
            include_bytes!("fixtures/legacy-texture.fbx").as_slice(),
        ),
        (
            "legacy big-endian take",
            include_bytes!("fixtures/legacy-be.fbx").as_slice(),
        ),
        (
            "legacy 5800 container",
            include_bytes!("fixtures/legacy-v5-binary.fbx").as_slice(),
        ),
    ] {
        sweep(name, true, || fbx_rs::load(input, &options));
    }
    let scene = fbx_rs::load(include_bytes!("fixtures/legacy-be.fbx"), &options).unwrap();
    sweep("legacy take evaluation", true, || {
        scene.evaluate(Some(0), 0.25)
    });
    sweep("legacy take bake", true, || {
        scene.bake_animation(Some(0), &fbx_rs::BakeOptions::default())
    });
}

#[test]
fn out_of_creation_order_bake_sort_allocator_refusals_are_typed() {
    let _serial = SERIAL.lock().unwrap();
    let mut input = String::from("; FBX 7.4.0\nObjects: {\n");
    // Creation order is parent-before-child; public breadth order moves the
    // second root ahead of the first root's child. Each has many properties
    // with equal creation rank, so the final sort must also preserve ties.
    for id in 1..=3 {
        writeln!(
            input,
            "Model: {id}, \"Model::N{id}\", \"Null\" {{\nProperties70: {{"
        )
        .unwrap();
        for prop in 0..1024 {
            writeln!(input, "P: \"Custom{prop:04}\", \"Number\", \"\", \"A\", 0").unwrap();
        }
        input.push_str("}\n}\n");
    }
    input.push_str("AnimationCurve: 10, \"AnimCurve::C\", \"\" {\nKeyTime: *2 { a: 0,46186158000 }\nKeyValueFloat: *2 { a: 0,1 }\nKeyAttrFlags: *1 { a: 4 }\nKeyAttrDataFloat: *4 { a: 0,0,0,0 }\nKeyAttrRefCount: *1 { a: 2 }\n}\nAnimationCurveNode: 11, \"AnimCurveNode::C\", \"\" {}\nAnimationLayer: 12, \"AnimLayer::L\", \"\" {}\nAnimationStack: 13, \"AnimStack::S\", \"\" {}\n}\nConnections: {\nC: \"OO\",1,0\nC: \"OO\",2,1\nC: \"OO\",3,0\nC: \"OP\",10,11,\"d|X\"\nC: \"OO\",11,12\nC: \"OO\",12,13\n");
    for id in 1..=3 {
        for prop in (0..1024).rev() {
            writeln!(input, "C: \"OP\",11,{id},\"Custom{prop:04}\"").unwrap();
        }
    }
    input.push_str("}\n");
    let scene = fbx_rs::load(input.as_bytes(), &Default::default()).unwrap();
    let options = fbx_rs::BakeOptions::default();
    SWEEP.with(|s| s.set(Some((0, usize::MAX))));
    let baseline = scene.bake_animation(None, &options);
    let total = SWEEP.with(|s| s.replace(None).unwrap().0);
    let baseline = baseline.unwrap();
    assert_eq!(baseline.properties.len(), 3072);
    for (i, prop) in baseline.properties.iter().enumerate() {
        assert_eq!(prop.source_id, i64::try_from(i / 1024 + 1).unwrap());
        assert_eq!(prop.name, format!("Custom{:04}", i % 1024).as_bytes());
    }
    // The final allocation is the public property buffer. Refuse it and the
    // preceding tail (including stable-sort scratch in the old implementation).
    // Thousands of entries force the old stable sort past its stack buffer.
    for n in total.saturating_sub(8)..total {
        SWEEP.with(|s| s.set(Some((0, n))));
        let result = scene.bake_animation(None, &options);
        SWEEP.with(|s| s.set(None));
        assert!(
            matches!(result, Err(fbx_rs::Error::Allocation)),
            "refusal {n}/{total}"
        );
    }
}

#[test]
fn invalid_mesh_warning_allocator_refusals_are_typed() {
    let _serial = SERIAL.lock().unwrap();
    let input = br#"; FBX 7.4.0
Objects: {
 Geometry: 1, "Geometry::Invalid", "Mesh" {
  Vertices: *4 { a: 0,0,0,1 }
  PolygonVertexIndex: *3 { a: 0,0,-1 }
 }
}
"#;
    let options = fbx_rs::LoadOptions::default();
    let baseline = fbx_rs::load(input, &options).unwrap();
    assert!(!baseline.warnings().is_empty());
    sweep("invalid mesh warnings", true, || {
        fbx_rs::load(input, &options)
    });
}
