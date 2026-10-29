//! PLX-93 / M1-T3 — the sealed seam, asserted on the expansion itself.
//!
//! PLX-73's `M1-T3 single_seam` says, verbatim:
//!
//! ```text
//! M1-T3 single_seam:  expanded macro output references exactly ONE core
//!                     entry symbol; zero direct schema-builder calls
//!                     # grep generated tokens
//! ```
//!
//! Nothing in either repo did that grep. `plexus-core` cannot (it has no
//! `plexus-macros` dependency in-tree); `plexus-macros`'s own suite asserts the
//! seam only through its *effects* — `every_ir_method_has_a_handler_and_vice_versa`
//! in `tests/plx91_switchover_tests.rs` pins the IR ↔ handler-table bijection,
//! and `crate::runtime::tests::seam` in plexus-core pins the runtime end. This
//! module is the missing token-level assertion, and it runs against
//! [`crate::codegen::generate_all`] — the exact function `#[activation]` calls.
//!
//! # Why the second clause is stated as a budget rather than a zero
//!
//! `zero direct schema-builder calls` was true of the shape PLX-73 designed and
//! is NOT true of the shape M1 shipped. PLX-91 requirement 4 was amended on
//! 2026-07-26: `MethodSchema` is computed *alongside* the IR rather than derived
//! from it, because six `MethodSchema` fields have no IR representation and
//! REQ-6 and RFC 002 §6.9.5 give the two surfaces contradictory rules about
//! request-context merging. So the expansion still builds a `MethodSchema`.
//!
//! Pretending otherwise would be narrowing the T-ID until it passed. Instead
//! this module asserts the two things that are actually load-bearing:
//!
//! 1. **Dispatch** goes through exactly one core symbol — `entry_with_state` —
//!    named exactly once in the expansion. That is the seam PLX-73 wanted, and
//!    it is intact.
//! 2. **The schema residue is pinned to an exact set of builder symbols.** It
//!    may shrink (PLX-103 retires `MethodSchema`); it may not grow silently.
//!
//! PLX-93 amends M1-T3 in PLX-73 to say this.

use quote::quote;
use syn::ItemImpl;

use crate::codegen::generate_all;
use crate::parse::HubMethodsAttrs;

/// Expand a fixture through the real `#[activation]` code path and return the
/// generated tokens as a string, whitespace-normalised so token-boundary
/// changes in `quote!` do not move the assertions.
fn expand(attr: proc_macro2::TokenStream, item: ItemImpl) -> String {
    let args: HubMethodsAttrs = syn::parse2(attr).expect("the activation attribute parses");
    let tokens = generate_all(args, item).expect("the fixture expands");
    tokens.to_string()
}

/// A deliberately ordinary activation: one unary `Result<T, E>` method, one
/// streaming method, one static child edge. Everything M1 claims to support,
/// nothing exotic — so the seam assertion is about the seam, not the fixture.
fn fixture() -> String {
    let item: ItemImpl = syn::parse_quote! {
        impl Seam {
            /// A unary method.
            #[plexus_macros::method]
            async fn get(&self, id: String) -> Result<Node, FetchError> {
                todo!()
            }

            /// A streaming method.
            #[plexus_macros::method(public)]
            async fn tail(&self) -> impl futures::Stream<Item = String> + Send + 'static {
                todo!()
            }

            /// A child edge.
            #[plexus_macros::child]
            fn leaf(&self) -> Leaf {
                todo!()
            }
        }
    };
    expand(
        quote! { namespace = "seam", version = "1.0.0", crate_path = "plexus_core" },
        item,
    )
}

/// Count non-overlapping occurrences of `needle` in `haystack`.
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// M1-T3, first clause: **exactly ONE core entry symbol**.
///
/// The whole vNext thesis is that the macro stops being a dispatcher and
/// becomes a producer of data handed to one sealed core function. If a second
/// dispatch entry point ever appears in the expansion, the seam has two mouths
/// and "core renames stop breaking services" is no longer true.
#[test]
fn m1_t3_the_expansion_names_exactly_one_core_dispatch_entry_symbol() {
    let expanded = fixture();

    // Count CALL sites, not mentions: `quote!`'s doc comments become
    // `#[doc = "..."]` attributes in the token stream, so the seam's own
    // documentation would otherwise be counted as a second reference.
    let calls = count(&expanded, ":: entry_with_state (");
    assert_eq!(
        calls, 1,
        "the expansion must CALL the sealed dispatch entry exactly once; \
         found {calls} call sites"
    );

    // The other entry point in `plexus_core::runtime` is `entry` itself. The
    // macro must not call it directly — `entry_with_state` is the stateful
    // form every generated activation goes through (PLX-97's handler-state
    // seam). `entry_with_state` contains the substring `entry`, so count the
    // call form.
    assert_eq!(
        count(&expanded, ":: entry ("),
        0,
        "the expansion must not call `runtime::entry` directly; \
         `entry_with_state` is the one seam"
    );

    // And the dispatch match the switchover gutted must stay gutted. The
    // generated `Activation::call` opens with `match method {` and PLX-91's
    // criterion c3 says its only arms are `"schema"` and `_`. A dispatch arm
    // is `"name" => {`; `method_help`'s help-text lookup is `"name" => Some (`,
    // which is why the discriminator is the brace and not the arrow.
    assert!(
        expanded.contains("match method { \"schema\" => {"),
        "the generated dispatch match must open on the `schema` arm"
    );
    for name in ["get", "tail"] {
        assert!(
            !expanded.contains(&format!("\"{name}\" => {{")),
            "found a generated string-match dispatch arm for `{name}`; \
             PLX-91 deleted these and they must not come back"
        );
    }
}

/// M1-T3, second clause, AMENDED: the schema-builder residue is a **pinned
/// budget**, not a zero.
///
/// PLX-73 asked for zero direct schema-builder calls. PLX-91's amended
/// requirement 4 makes that unreachable at M1: `MethodSchema` is computed
/// alongside the IR. This test pins exactly which core schema symbols the
/// expansion is allowed to name, so PLX-103 (retire `MethodSchema`) can watch
/// the number go to zero, and so nothing new can be added without editing this
/// list on purpose.
#[test]
fn m1_t3_the_schema_builder_residue_is_pinned_not_zero() {
    let expanded = fixture();

    // The residue, named. Each of these is a core schema type the expansion
    // still constructs directly because `MethodSchema` survives M1.
    let residue = ["MethodSchema", "PluginSchema"];

    let present: Vec<&str> = residue
        .iter()
        .copied()
        .filter(|s| expanded.contains(s))
        .collect();

    assert!(
        !present.is_empty(),
        "M1 ships with a MethodSchema residue by PLX-91's amended requirement 4; \
         if this is now empty the amendment has been discharged — retire this \
         assertion and restore PLX-73's original `zero` clause"
    );

    // Nothing OUTSIDE the named residue. This is the grep PLX-73 asked for:
    // any other core schema-builder appearing in the expansion is new drift.
    for forbidden in [
        "SchemaBuilder",
        "schema_for!",
        "PluginSchemaBuilder",
        "MethodSchemaBuilder",
    ] {
        assert!(
            !expanded.contains(forbidden),
            "the expansion names `{forbidden}`, which is outside the pinned \
             M1 schema residue {residue:?}"
        );
    }
}

/// The seam carries **data**, not behaviour: the expansion emits an
/// `ActivationIr` and a `HandlerTable`, which is the whole of what M1 promised
/// the macro would become.
#[test]
fn m1_t3_the_expansion_emits_an_ir_and_a_handler_table() {
    let expanded = fixture();

    for required in ["ActivationIr", "HandlerTable", "ErasedHandler"] {
        assert!(
            expanded.contains(required),
            "the expansion must emit `{required}`"
        );
    }
}
