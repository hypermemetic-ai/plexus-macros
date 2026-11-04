//! RETURNS-46 review: a dynamic instance's path is the client's own — no
//! process-wide set, nothing leaked, no lock on the accessor.
//!
//! A counting allocator: 1M distinct names, and 100k × 10 nested ones, leave
//! the live bytes where they were once the clients drop.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use aggregate_probe::*;

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
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

#[test]
fn the_accessor_takes_no_lock() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // With a process-wide lock, eight threads take longer than one doing
    // all their work; without one, they take less.
    let f = Arc::new(Fast(0));
    let per = 200_000;
    let serial = {
        let root = RootClient::new(&*f);
        let t0 = std::time::Instant::now();
        for _ in 0..8 * per {
            let _ = root.org("acme").path().len();
        }
        t0.elapsed()
    };
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..8 {
            let f = f.clone();
            s.spawn(move || {
                let root = RootClient::new(&*f);
                for _ in 0..per {
                    let _ = root.org("acme").path().len();
                }
            });
        }
    });
    let parallel = t0.elapsed();
    println!("8 x {per} org(\"acme\"): serial {serial:?}, 8 threads {parallel:?}");
    assert!(parallel < serial, "8 threads {parallel:?} not faster than one {serial:?}");
}
