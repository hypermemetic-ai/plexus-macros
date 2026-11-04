//! RETURNS-46 review: a dynamic instance's path is the client's own — no
//! process-wide set, nothing leaked, no lock on the accessor.
//!
//! A counting allocator: 1M distinct names, and 100k × 10 nested ones, leave
//! the live bytes where they were once the clients drop; an accessor
//! allocates its path and name and nothing else, and its code holds no lock.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use aggregate_probe::*;

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static ALLOCS: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        System.dealloc(p, l)
    }
}

#[global_allocator]
static A: Counting = Counting;

/// The count is process-wide: the tests here take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_million_names_leave_memory_flat() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fast(0);
    let root = RootClient::new(&f);
    // warm anything lazily allocated once
    let _ = root.org("warm").path().len();
    let before = LIVE.load(Ordering::Relaxed);
    for i in 0..1_000_000u32 {
        let name = format!("org-{i:08}");
        assert_eq!(root.org(&name).path().len(), "root.".len() + name.len());
    }
    for i in 0..100_000u32 {
        let org = root.org(&format!("org-{i:08}"));
        for j in 0..10 {
            let _ = org.repo(&format!("repo-{j}")).path().len();
        }
    }
    let after = LIVE.load(Ordering::Relaxed);
    assert!(after - before < 4096, "live bytes grew by {} after 2M dropped clients", after - before);
}

/// The path an accessor builds takes no lock: its code (plexus-client's
/// `route.rs`, `RoutePath` in `transport.rs`) holds no lock type and no
/// process-wide state — and an accessor allocates exactly the instance's
/// path and its name, both freed with the client.
#[test]
fn the_accessor_takes_no_lock() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let route = include_str!("../../../plexus-client/src/route.rs");
    let transport = include_str!("../../../plexus-client/src/transport.rs");
    let path_code = &transport[transport.find("pub struct RoutePath").unwrap()..transport.find("/// A fully qualified Plexus method").unwrap()];
    for (file, code) in [("route.rs", route), ("transport.rs RoutePath", path_code)] {
        for word in ["Mutex", "RwLock", "OnceLock", "LazyLock", "static ", "thread_local", "Atomic"] {
            assert!(!code.contains(word), "{file} holds `{word}`");
        }
    }
    let f = Fast(0);
    let root = RootClient::new(&f);
    let before = ALLOCS.load(Ordering::Relaxed);
    let org = root.org("acme");
    let during = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(during - before, 2, "the instance's path and its name, nothing else");
    drop(org);
}
