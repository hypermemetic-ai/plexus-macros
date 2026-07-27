//! PLX-93 / M1-T7, AMENDED — the surface that was ACTUALLY removed.
//!
//! PLX-73 M1-T7 named four things:
//!
//! ```text
//! M1-T7 removed_surface: credentials-derive / scope / public / bidirectional
//!                        → compile error pointing at ext(...) or the
//!                        quarantine crate
//! ```
//!
//! Three of the four did not happen, and each has a recorded reason:
//!
//! - **`bidirectional`** was REVERSED inside PLX-73 itself on 2026-07-25 —
//!   it is promoted to a first-class `MethodShape` because ACP (PLX-76) is
//!   built on it. PLX-92 c3 confirms 48 references retained with live
//!   consumers in plexus-substrate, plexus-codegen and plexus-transport.
//! - **`scope` / `public`** were NOT deleted. PLX-92 c1 ran a fresh census at
//!   deletion time and found live consumers the 2026-07-25 snapshot had
//!   missed: plexus-core's own `scope_gate_dispatch` and `site_hint_emission`
//!   suites, and `public` is the only suppression for the RED-6 default-deny
//!   error. They stay on the `#[activation]` grammar.
//!
//! What WAS removed is the **credentials derive** (1,090 lines, zero
//! consumers) and the 0.5 **`StreamEvent` derive** — and neither had a test.
//! This fixture is that test: both must fail to resolve, loudly.
//!
//! The `#[method(bidirectional)]` / `scope` / `public` half of the original
//! T-ID is covered where it is actually true — on the vNext-only `method_ir!`
//! grammar — by `removed_surface_keys_point_at_their_replacement` in
//! `src/ir_parse/tests.rs`.

#[derive(plexus_macros::Credentials)]
struct GatewayCreds {
    token: String,
}

#[derive(plexus_macros::StreamEvent)]
enum Event {
    Tick,
}

fn main() {}
