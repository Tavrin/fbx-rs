//! Check the host byte envelope against actual live allocator requests.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static LIVE: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}
struct Counter;
fn change(old: usize, new: usize) {
    let _ = LIVE.try_with(|cell| {
        if let Some((live, peak)) = cell.get() {
            let live = live.checked_sub(old).unwrap().checked_add(new).unwrap();
            cell.set(Some((live, peak.max(live))));
        }
    });
}
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            change(0, layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            change(0, layout.size());
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(p, layout, size) };
        if !next.is_null() {
            change(layout.size(), size);
        }
        next
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        change(layout.size(), 0);
    }
}
#[global_allocator]
static ALLOC: Counter = Counter;

#[test]
fn peak_live_requests_fit_the_shared_load_envelope() {
    let mut many = String::from("; FBX 7.4.0 project file\nObjects: {\n");
    use std::fmt::Write;
    for id in 1..=300 {
        writeln!(many, "Model: {id}, \"Model::N{id}\", \"Null\" {{ }}").unwrap();
    }
    many.push_str("}\n");
    let inputs = [
        include_bytes!("fixtures/allocation-rich.fbx").as_slice(),
        include_bytes!("fixtures/allocation-rich-binary.fbx").as_slice(),
        include_bytes!("fixtures/legacy-deform.fbx").as_slice(),
        include_bytes!("fixtures/legacy-take.fbx").as_slice(),
        many.as_bytes(),
    ];
    for input in inputs {
        fbx_rs::load(input, &fbx_rs::LoadOptions::default()).unwrap();
        let mut accepted = false;
        for max_allocation in [1, 4096, 16384, 65536, 262144, 1048576, 4194304] {
            let limits = fbx_rs::Limits {
                max_allocation,
                ..Default::default()
            };
            LIVE.with(|cell| cell.set(Some((0, 0))));
            let result = fbx_rs::load_with_limits(input, &fbx_rs::LoadOptions::default(), &limits);
            let valid_result = result.is_ok()
                || matches!(
                    result,
                    Err(fbx_rs::Error::LimitExceeded(fbx_rs::Limit::Allocation))
                );
            accepted |= result.is_ok();
            drop(result);
            let (live, peak) = LIVE.with(|cell| cell.replace(None).unwrap());
            assert_eq!(live, 0);
            assert!(valid_result, "unexpected error at {max_allocation} bytes");
            assert!(
                peak <= max_allocation,
                "peak {peak} exceeds envelope {max_allocation}"
            );
        }
        assert!(accepted);
    }
}
