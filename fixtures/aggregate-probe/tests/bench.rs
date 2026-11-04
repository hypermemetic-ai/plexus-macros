//! RETURNS-46 review: the per-event cost of `.events()` — the root's own
//! call, one static level, two dynamic levels — 2M events, three rounds.
//! `cargo test --release -p aggregate-probe --test bench -- --ignored --nocapture`.

use futures::StreamExt;
use aggregate_probe::*;

fn run<S: futures::Stream + Unpin>(s: S) -> usize {
    futures::executor::block_on(s.count())
}

#[test]
#[ignore]
fn bench_events() {
    let items = 2_000_000u32;
    let f = Fast(items);
    let root = RootClient::new(&f);
    for round in 0..3 {
        let t0 = std::time::Instant::now();
        let n = run(root.tick(items).events());
        let a = t0.elapsed();
        let t1 = std::time::Instant::now();
        let n2 = run(root.left().tick(items).events());
        let b = t1.elapsed();
        let org = root.org("acme");
        let repo = org.repo("r");
        let t2 = std::time::Instant::now();
        let n3 = run(repo.tick(items).events());
        let c = t2.elapsed();
        assert_eq!((n2, n3), (n, n));
        let ns = |d: std::time::Duration| d.as_nanos() as f64 / items as f64;
        println!("round {round}: root.tick {:.1} ns/ev; child (left) {:.1} ns/ev; org.repo (2 dyn levels) {:.1} ns/ev", ns(a), ns(b), ns(c));
    }
}
