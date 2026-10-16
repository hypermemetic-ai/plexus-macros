//! MACRO-13 · the `server` feature default, built.
//!
//! `fixtures/macro13-echo` declares a `server` feature and writes no
//! `server_cfg` on `EchoActivation`; `HostActivation` writes
//! `server_cfg = "feature = \"host\""`. A feature set is a build, so these
//! run cargo on the fixture (in a target directory of their own, so they do
//! not wait on the one running this test):
//!
//! - `--no-default-features`: the traits and the clients build, and
//!   plexus-core is nowhere in the tree;
//! - `--no-default-features --features server`: `EchoActivation`'s server
//!   half builds, and `HostActivation`'s — whose struct exists only under
//!   `host` — is gated on `host`, or this build would fail.

use std::path::PathBuf;
use std::process::Command;

fn cargo(args: &[&str]) -> std::process::Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    Command::new(cargo)
        .args(args)
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target").join("macro13-features"))
        .output()
        .expect("cargo runs")
}

fn assert_builds(args: &[&str]) {
    let out = cargo(args);
    assert!(
        out.status.success(),
        "cargo {} failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn without_the_server_feature_the_trait_and_client_build_with_no_plexus_core() {
    assert_builds(&["build", "-p", "macro13-echo", "--no-default-features"]);

    let out = cargo(&["tree", "-p", "macro13-echo", "--no-default-features", "-e", "normal", "--prefix", "none"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tree = String::from_utf8_lossy(&out.stdout);
    for absent in ["plexus-core ", "jsonrpsee ", "tokio "] {
        assert!(!tree.lines().any(|l| l.starts_with(absent)), "{absent}in the tree:\n{tree}");
    }
    assert!(tree.lines().any(|l| l.starts_with("plexus-client ")), "{tree}");
}

#[test]
fn server_cfg_overrides_the_default_gate() {
    assert_builds(&["build", "-p", "macro13-echo", "--no-default-features", "--features", "server"]);
}
