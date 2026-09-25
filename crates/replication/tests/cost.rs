//! `Replicated::cost` must track the heap a store really takes, so a host
//! that limits a guest's store by cost limits its memory. This binary counts
//! every allocation and compares.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use pylon_replication::Replicated;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Heap the store built by `fill` takes, and its cost.
fn measure(fill: impl FnOnce(&mut Replicated)) -> (usize, usize) {
    let before = LIVE.load(Ordering::Relaxed);
    let mut store = Replicated::new();
    fill(&mut store);
    let heap = (LIVE.load(Ordering::Relaxed) - before) as usize;
    let cost = store.cost();
    drop(store);
    (heap, cost)
}

type Fill = Box<dyn FnOnce(&mut Replicated)>;

// One test, so no other test allocates while it measures.
#[test]
fn cost_is_close_to_the_real_heap() {
    let loads: Vec<(&str, Fill)> = vec![
        (
            "bare entities",
            Box::new(|s| {
                for id in 0..100_000 {
                    s.spawn(id, [0.0; 3]);
                }
            }),
        ),
        (
            "one empty component each",
            Box::new(|s| {
                for id in 0..100_000 {
                    s.spawn(id, [0.0; 3]);
                    s.set_component(id, 1, &[]);
                }
            }),
        ),
        (
            "256 tombstones each",
            Box::new(|s| {
                for id in 0..5_000 {
                    s.spawn(id, [0.0; 3]);
                    for c in 0..=255u8 {
                        s.set_component(id, c, &[]);
                        s.remove_component(id, c);
                    }
                }
            }),
        ),
        (
            "four 32-byte components each",
            Box::new(|s| {
                for id in 0..20_000 {
                    s.spawn(id, [0.0; 3]);
                    for c in 0..4u8 {
                        s.set_component(id, c, &[c; 32]);
                    }
                }
            }),
        ),
    ];
    for (name, fill) in loads {
        let (heap, cost) = measure(fill);
        let ratio = heap as f64 / cost as f64;
        println!("{name}: heap {heap}, cost {cost}, ratio {ratio:.2}");
        assert!(
            ratio <= 1.5,
            "{name}: the store takes {heap} bytes of heap but costs {cost} ({ratio:.2}x)"
        );
    }
}
