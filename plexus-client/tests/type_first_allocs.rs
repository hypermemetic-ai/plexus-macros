//! PERF.115-43: an event read with its `type` first allocates what its fields
//! hold and nothing more; serde's read of it allocates its buffer too. Its
//! own test binary, for the counting allocator.

use serde::Deserialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// The system allocator, counting the allocations this thread makes.
struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: each call is the system allocator's own, with its own arguments;
// the count is a thread-local `Cell` with no destructor, initialised as a
// constant, so counting allocates nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// How many allocations `read` makes, its result dropped after the count.
fn allocations<T>(read: impl FnOnce() -> T) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    let read = std::hint::black_box(read());
    let made = ALLOCATIONS.with(Cell::get) - before;
    drop(read);
    made
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum Buffered {
    Item { n: u64, pad: String },
    Ready,
}

#[derive(Debug, plexus_macros::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum TypeFirst {
    Item { n: u64, pad: String },
    Ready,
}

#[test]
fn an_event_read_with_its_type_first_allocates_only_what_its_fields_hold() {
    let item = r#"{"type":"item","n":4321,"pad":"xxxxxxxxxxxxxxxx"}"#;
    // serde's read: its buffer, then the pad's String.
    assert_eq!(allocations(|| serde_json::from_str::<Buffered>(item).unwrap()), 2);
    // The type first: the pad's String.
    assert_eq!(allocations(|| serde_json::from_str::<TypeFirst>(item).unwrap()), 1);
    // A unit variant with keys beside its tag: nothing at all.
    let ready = r#"{"type":"ready","n":1,"pad":"x"}"#;
    assert_eq!(allocations(|| serde_json::from_str::<Buffered>(ready).unwrap()), 1);
    assert_eq!(allocations(|| serde_json::from_str::<TypeFirst>(ready).unwrap()), 0);
    // The type last: serde's read, buffer and all, as before.
    let last = r#"{"n":4321,"pad":"xxxxxxxxxxxxxxxx","type":"item"}"#;
    assert_eq!(allocations(|| serde_json::from_str::<TypeFirst>(last).unwrap()), 2);
}
