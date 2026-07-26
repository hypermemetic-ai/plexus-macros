//! Strict `#[method(...)]` attribute parsing for the vNext parser.
//!
//! # Two passes, one of them borrowed
//!
//! **Pass 1 (here) — strictness.** vNext *reverses* the old path's
//! forward-compatibility posture: an unrecognized key inside `#[method(...)]` is
//! a span-accurate error naming the key, not a silently ignored token
//! (PLX-73 trap closures; M1-T6). `ext(...)` is the escape hatch, and the error
//! says so. Keys belonging to surfaces vNext removed or promoted
//! (`bidirectional`, `scope`, `public`, `streaming`, `returns`, `request`,
//! `override_call`, `credential`) get a tailored message pointing at their
//! replacement rather than a bare "unknown key" — that is M1-T7's
//! "removed surface must fail loud".
//!
//! This pass also reads `http_method` **as the real enum**: the literal is
//! matched against the five verbs and mapped straight to a
//! [`HttpMethodIrSpec`] variant. There is no `String` field to convert later and
//! therefore no downstream "should not happen" fallback arm — the only
//! non-matching case is a user error, and it is reported here at the literal.
//!
//! **Pass 2 — reuse.** Everything else the old parser already extracts
//! correctly (`name`, `description`, `params(...)` docs, and the whole PLX-71
//! `ext(...)` grammar with its duplicate-key and literal-kind errors) is
//! obtained by running [`crate::parse::HubMethodAttrs`]'s own `Parse` impl over
//! the same token stream. The `ext(...)` parsing is *not* re-implemented — this
//! is a read-only call into the existing code, so there is exactly one
//! implementation of PLX-71 semantics in the crate.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, ExprLit, Lit, Meta, MetaNameValue, Token};

use super::spec::HttpMethodIrSpec;
use crate::parse::{HubMethodAttrs, MethodRequestOverride};

/// The vNext-recognized `#[method(...)]` keys. Everything else is an error.
const ALLOWED_KEYS: &[&str] = &[
    "name",
    "description",
    "params",
    "ext",
    "http_method",
    // RFC 002 §6.9 — the per-method request-context override. Reinstated by
    // PLX-90: `request` was removed from vNext when it meant "the activation's
    // request struct is spliced into the parameter list", and it is back with
    // the *opposite* meaning — a declaration of the transport facts the method
    // depends on, which §6.9.5 forbids merging into the parameters.
    "request",
];

/// Keys whose surface vNext removed or relocated, with where to go instead.
fn removed_surface_hint(key: &str) -> Option<&'static str> {
    Some(match key {
        "bidirectional" => {
            "`bidirectional` is gone: callbacks are declared by the method's \
             capability handle. Take a `client: Client<(Permission, FsRead)>` \
             parameter — `MethodIr::callbacks` is derived from that type"
        }
        "streaming" => {
            "`streaming` is gone: a turn streams iff it declares an update item \
             type. Return `impl Stream<Item = T>` — `MethodIr::updates` (and \
             `is_streaming`) is derived from the return type"
        }
        "scope" | "public" => {
            "`scope` / `public` are gone from `#[method(...)]`: authorization is \
             `MethodIr::auth`, derived from the method's `auth:` parameter \
             (default-deny). Carry any extra policy in `ext(...)`"
        }
        "returns" => {
            "`returns(...)` variant filtering is gone: the turn envelope declares \
             an update item type and a terminal type directly from the signature"
        }
        "override_call" => "`override_call` is gone: return `impl Stream<Item = T>` instead",
        "credential" | "credentials" => {
            "the credentials surface is gone (zero consumers, PLX-73); carry \
             gateway metadata in `ext(...)`"
        }
        _ => return None,
    })
}

/// How strictly a `#[method(...)]` list is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// PLX-79's posture: an unrecognized key is a span-accurate error.
    /// This is what `method_ir!` uses.
    Strict,
    /// PLX-91's switchover posture: the legacy `#[method(...)]` grammar is
    /// accepted, because `#[activation]` must read the corpus that exists.
    Legacy,
}

/// The subset of `#[method(...)]` the vNext parser consumes.
#[derive(Debug, Default)]
pub(crate) struct MethodAttrs {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) param_docs: HashMap<String, String>,
    /// `None` means "not declared"; the IR's own default (`POST`) applies.
    pub(crate) http_method: Option<HttpMethodIrSpec>,
    /// RFC 002 §6.9.2 — the method's own request-context declaration, as
    /// tokens. `Some` REPLACES the owning activation's declaration for this
    /// method; `None` means the method declares none of its own and §6.9.3
    /// resolution falls back to the activation's.
    pub(crate) request_context: Option<syn::Type>,
    pub(crate) extensions: Vec<(String, serde_json::Value)>,
}

impl MethodAttrs {
    /// Find the `#[method(...)]` / `#[plexus_macros::method(...)]` attribute on
    /// an item and parse it. A method with no such attribute (or a bare
    /// `#[method]`) yields defaults.
    /// PLX-91: the same parse, with the strictness dial exposed.
    ///
    /// `method_ir!` keeps [`Mode::Strict`] — that is PLX-79's acceptance
    /// criterion #3 and its trybuild fixture. `#[activation]` uses
    /// [`Mode::Legacy`], because the switchover must read the grammar the
    /// existing corpus is written in: `public`, `scope = "..."`, `streaming`
    /// and `request = ()` are legacy `#[method(...)]` keys that PLX-79
    /// deliberately rejects, and rejecting them from `#[activation]` would
    /// fail to compile 14 `method(public)` and 12 `scope = …` sites in this
    /// crate's own suite alone. Their *meaning* is not lost — it is read off
    /// [`crate::parse::MethodInfo`], which parses the same tokens, and folded
    /// into the IR by the caller (see `codegen::mod::activation_ir_fn`).
    pub(crate) fn parse_from_with_mode(attrs: &[Attribute], mode: Mode) -> syn::Result<Self> {
        let Some(attr) = attrs.iter().find(|a| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "method")
        }) else {
            return Ok(Self::default());
        };

        let tokens = match &attr.meta {
            Meta::Path(_) => TokenStream::new(),
            Meta::List(list) => list.tokens.clone(),
            Meta::NameValue(nv) => {
                return Err(syn::Error::new_spanned(
                    nv,
                    "#[method] takes a parenthesized argument list, e.g. \
                     #[method(http_method = \"GET\")]",
                ))
            }
        };

        Self::parse_tokens_with_mode(tokens, mode)
    }

    /// Parse an already-extracted `#[method(...)]` argument token stream.
    pub(crate) fn parse_tokens_with_mode(tokens: TokenStream, mode: Mode) -> syn::Result<Self> {
        // ---- Pass 1: strictness + the typed http_method ----
        let mut http_method = None;
        let mut request_span = None;
        if !tokens.is_empty() {
            let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
            let metas = syn::parse::Parser::parse2(parser, tokens.clone())?;
            for meta in &metas {
                let path = meta.path();
                let key = path
                    .get_ident()
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| quote::ToTokens::to_token_stream(path).to_string());

                if mode == Mode::Strict && !ALLOWED_KEYS.contains(&key.as_str()) {
                    let message = match removed_surface_hint(&key) {
                        Some(hint) => format!("unknown `#[method(...)]` key `{key}`: {hint}"),
                        None => format!(
                            "unknown `#[method(...)]` key `{key}`. vNext accepts only: {}. \
                             Unknown keys are errors by design — use `ext({key} = ...)` for \
                             open extension metadata",
                            ALLOWED_KEYS.join(", "),
                        ),
                    };
                    return Err(syn::Error::new(path.span(), message));
                }

                if key == "http_method" {
                    http_method = Some(parse_http_method(meta)?);
                }
                if key == "request" {
                    request_span = Some(path.span());
                }
            }
        }

        // ---- Pass 2: reuse the existing parser for everything shared ----
        let base: HubMethodAttrs = syn::parse2(tokens)?;

        // RFC 002 §6.9.2 — the method-level declaration, read out of the
        // *existing* parser's `request_override` rather than re-parsed here, so
        // there is one implementation of `request = <Type>` in the crate.
        let request_context = match base.request_override {
            None => None,
            Some(MethodRequestOverride::Type(ty)) => Some(ty),
            // PLX-91: on the legacy path `request = ()` keeps its legacy
            // meaning ("skip activation-level extraction"), which is a
            // *dispatch* instruction with no IR spelling. The IR records the
            // absence; the handler still honours the skip.
            Some(MethodRequestOverride::Skip) if mode == Mode::Legacy => None,
            Some(MethodRequestOverride::Skip) => {
                // `request = ()` meant "opt this method out of the activation's
                // request extraction" on the legacy path. RFC 002 §6.9 has no
                // wire form for it: §6.9.3 makes method-level absence *mean*
                // "inherit the activation's", so a method on a
                // context-declaring activation cannot say "I need nothing".
                // Inventing one here would be a parallel mechanism, which
                // PLX-90 forbids — so this fails loud and names the gap.
                return Err(syn::Error::new(
                    request_span.unwrap_or_else(proc_macro2::Span::call_site),
                    "`request = ()` has no meaning under CONNECTOME RFC 002 §6.9. \
                     A method's declaration REPLACES its activation's (§6.9.2) and \
                     method-level absence MEANS \"use the activation's\" (§6.9.3), so \
                     the IR has no way to spell \"this method needs no transport \
                     facts\" on an activation that declares some. Declare the narrower \
                     type explicitly (`request = MinimalCtx`), or drop the \
                     activation-level `request = ...` and declare it per method. \
                     (Reported upstream as an RFC 002 §6.9 finding.)",
                ));
            }
        };

        Ok(Self {
            name: base.name,
            description: base.description,
            param_docs: base.param_docs,
            http_method,
            request_context,
            extensions: base.extensions,
        })
    }
}

/// `http_method = "GET"` → the real enum variant. No `String` survives this
/// function, so no consumer needs a fallback arm.
fn parse_http_method(meta: &Meta) -> syn::Result<HttpMethodIrSpec> {
    let Meta::NameValue(MetaNameValue { value, .. }) = meta else {
        return Err(syn::Error::new_spanned(
            meta,
            "http_method expects a string literal, e.g. http_method = \"GET\"",
        ));
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            "http_method expects a string literal, e.g. http_method = \"GET\"",
        ));
    };

    match s.value().to_uppercase().as_str() {
        "GET" => Ok(HttpMethodIrSpec::Get),
        "POST" => Ok(HttpMethodIrSpec::Post),
        "PUT" => Ok(HttpMethodIrSpec::Put),
        "DELETE" => Ok(HttpMethodIrSpec::Delete),
        "PATCH" => Ok(HttpMethodIrSpec::Patch),
        other => Err(syn::Error::new(
            s.span(),
            format!("invalid HTTP method `{other}`. Valid: GET, POST, PUT, DELETE, PATCH"),
        )),
    }
}
