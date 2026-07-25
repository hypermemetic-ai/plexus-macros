//! PLX-79 (M1·B) — the vNext parser: a `#[method]`-annotated signature becomes
//! a `plexus_core::ir::MethodIr`, and an `#[activation]` impl block becomes an
//! `ActivationIr` (methods + child edges).
//!
//! # Additive by construction
//!
//! Nothing here is reachable from the existing `#[activation]` path. `parse.rs`'s
//! `HubMethodAttrs` / `MethodInfo` and `codegen/{activation,method_enum}.rs` are
//! untouched and keep emitting exactly what they emitted before; the switchover
//! is build F. Existing helpers are *reused read-only* — in particular the
//! PLX-71 `ext(...)` parsing, which is not re-implemented here: it is obtained
//! by running the existing [`crate::parse::HubMethodAttrs`] parser over the same
//! tokens (see [`attrs`]).
//!
//! # Tokens, not types — and how the IR is nevertheless produced
//!
//! A proc macro sees **tokens**. `MethodIr` is not a token-level value: three of
//! its fields (`params`, `updates`, `terminal`) hold a
//! `SchemaRef`, which by PLX-75's construction rule pairs a canonical type name
//! with a *resolved `schemars::Schema`* — and resolving a Rust type's JSON
//! Schema requires the type system, i.e. the consuming crate's compilation, not
//! macro expansion. `callbacks` is the same story one level up: the capability
//! set `C` in `Client<C>` is a type, and only rustc can decide what
//! `<C as CapabilitySet>::callbacks()` is.
//!
//! So the parser is split at the only seam that exists:
//!
//! 1. **Parse** (this module's [`spec`] types) — everything decidable from
//!    tokens: names, dotted id, descriptions, parameter list and optionality,
//!    which type is the update item and which is the terminal, `ext(...)`
//!    pairs, the HTTP verb *as the real enum*, the auth requirement, and the
//!    identity of the capability-set type. Type-level fields are held as
//!    [`syn::Type`], never as strings that get re-parsed later.
//! 2. **Emit** ([`emit`]) — a Rust *expression* that evaluates to the real
//!    `MethodIr` / `ActivationIr` in the consuming crate, where
//!    `schemars::schema_for!(T)` and `<C as CapabilitySet>::callbacks()` are
//!    resolvable. The capability tokens are handed straight back to rustc, so
//!    the descriptors come from `plexus-core`'s own
//!    `Capability::descriptor` and there
//!    is no capability registry of any kind in this crate.
//!
//! The expression is **fallible** — `Result<MethodIr, SchemaRefError>` — because
//! `SchemaRef::new` is fallible by design (PLX-75). It is not `.expect()`-ed
//! away: a parameter whose type resolves to an uninformative schema (e.g. a bare
//! `serde_json::Value`) must surface as an error, not a panic in a generated
//! path (PLX-73 trap closures).
//!
//! # Entry points
//!
//! [`method_ir_expr`] and [`activation_ir_expr`], exposed to consumers (and to
//! this crate's tests) through the `plexus_macros::method_ir!` /
//! `plexus_macros::activation_ir!` function-like macros registered in
//! `lib.rs`. Build F calls the same two functions from `#[activation]`.

pub(crate) mod attrs;
pub(crate) mod emit;
pub(crate) mod signature;
pub(crate) mod spec;

#[cfg(test)]
mod tests;

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{ItemImpl, LitStr, Token, TraitItemFn};

pub(crate) use spec::{ActivationIrSpec, MethodIrSpec};

/// `namespace = "ns", <fn>` — the input of `method_ir!`.
pub(crate) struct MethodIrInput {
    namespace: String,
    func: TraitItemFn,
}

impl Parse for MethodIrInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: syn::Ident = input.parse()?;
        if key != "namespace" {
            return Err(syn::Error::new_spanned(
                &key,
                "method_ir! expects `namespace = \"...\"` first, then the \
                 `#[method(...)]`-annotated function signature",
            ));
        }
        input.parse::<Token![=]>()?;
        let ns: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let func: TraitItemFn = input.parse()?;
        Ok(Self {
            namespace: ns.value(),
            func,
        })
    }
}

/// `namespace = "ns", version = "1.0.0"[, backend_name = ..][, respond_method =
/// ..][, description = ..], <impl block>` — the input of `activation_ir!`.
pub(crate) struct ActivationIrInput {
    namespace: String,
    version: String,
    backend_name: Option<String>,
    respond_method: Option<String>,
    description: Option<String>,
    item: ItemImpl,
}

impl Parse for ActivationIrInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut namespace: Option<String> = None;
        let mut version: Option<String> = None;
        let mut backend_name: Option<String> = None;
        let mut respond_method: Option<String> = None;
        let mut description: Option<String> = None;

        // Leading `key = "value",` pairs, terminated by the impl block.
        while input.peek(syn::Ident) && input.peek2(Token![=]) {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: LitStr = input.parse()?;
            input.parse::<Token![,]>()?;
            let slot = match key.to_string().as_str() {
                "namespace" => &mut namespace,
                "version" => &mut version,
                "backend_name" => &mut backend_name,
                "respond_method" => &mut respond_method,
                "description" => &mut description,
                other => {
                    return Err(syn::Error::new_spanned(
                        &key,
                        format!(
                            "unknown activation_ir! argument `{other}`; expected one of \
                             namespace, version, backend_name, respond_method, description"
                        ),
                    ))
                }
            };
            *slot = Some(value.value());
        }

        let item: ItemImpl = input.parse()?;
        let span = item.self_ty.as_ref();

        Ok(Self {
            namespace: namespace.ok_or_else(|| {
                syn::Error::new_spanned(span, "activation_ir! requires `namespace = \"...\"`")
            })?,
            version: version.ok_or_else(|| {
                syn::Error::new_spanned(span, "activation_ir! requires `version = \"...\"`")
            })?,
            backend_name,
            respond_method,
            description,
            item,
        })
    }
}

/// Parse one `#[method(...)]` signature and emit the expression that builds its
/// `MethodIr`.
///
/// The returned tokens have type
/// `Result<plexus_core::ir::MethodIr, plexus_core::ir::SchemaRefError>`.
pub(crate) fn method_ir_expr(input: MethodIrInput) -> syn::Result<TokenStream> {
    let spec = MethodIrSpec::from_signature(&input.namespace, &input.func)?;
    let paths = emit::Paths::resolve();
    let core = &paths.core;
    let body = emit::method_expr(&paths, &spec);
    Ok(quote! {
        (|| -> ::core::result::Result<#core::ir::MethodIr, #core::ir::SchemaRefError> {
            ::core::result::Result::Ok(#body)
        })()
    })
}

/// Parse an `#[activation]`-shaped impl block and emit the expression that
/// builds its `ActivationIr`, hashes included.
///
/// The returned tokens have type
/// `Result<plexus_core::ir::ActivationIr, plexus_core::ir::SchemaRefError>`.
pub(crate) fn activation_ir_expr(input: ActivationIrInput) -> syn::Result<TokenStream> {
    let spec = ActivationIrSpec::from_impl(
        &input.namespace,
        &input.version,
        input.backend_name.clone(),
        input.respond_method.clone(),
        input.description.clone(),
        &input.item,
    )?;
    let paths = emit::Paths::resolve();
    let core = &paths.core;
    let body = emit::activation_expr(&paths, &spec);
    Ok(quote! {
        (|| -> ::core::result::Result<#core::ir::ActivationIr, #core::ir::SchemaRefError> {
            #body
        })()
    })
}
