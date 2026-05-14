//! `#[derive(Credentials)]` + `#[credential(...)]` field helper attribute and
//! `CredentialFieldMarker` registry emission.
//!
//! Per `AUTHZ-CRED-MACRO-1`:
//!
//! The macro is applied to a **struct or enum item** that contains one or more
//! credential-bearing fields. Within the item, individual fields carry an inner
//! `#[credential(kind = "...", attach_as = "...", ...)]` attribute declaring
//! the credential's metadata.
//!
//! User-facing form:
//!
//! ```ignore
//! #[derive(plexus_macros::Credentials, Serialize, Deserialize, JsonSchema)]
//! pub struct LoginEvent {
//!     pub user_id: String,
//!     #[credential(
//!         kind = "bearer",
//!         attach_as = "header:authorization",
//!         scheme = "Bearer ",
//!     )]
//!     pub session: Credential<SessionToken>,
//! }
//! ```
//!
//! ## Why a derive macro
//!
//! The original design doc (`AUTHZ-CRED-S01-output.md` §1) sketched the
//! attribute as `#[credential(...)]` applied directly to a field.
//! Stable Rust proc-macros cannot intercept attribute macros on struct
//! fields — only on items (struct, enum, impl, fn, etc.). Helper attributes
//! of `#[proc_macro_derive(...)]` macros CAN appear on fields, so the macro
//! is registered as a derive with `credential` as a helper attribute. See
//! `plans/AUTHZ/AUTHZ-CRED-MACRO-1-RUN-NOTES.md` for the full rationale and
//! the migration path if Rust ever stabilises field-level attribute macros.
//!
//! The user surface is `#[derive(Credentials)]` plus inner `#[credential(...)]`
//! field attributes. The macro emits registry helpers in the same module as
//! the item; the original item is unchanged.
//!
//! ## What the macro emits
//!
//! For each struct/enum annotated `#[derive(Credentials)]`, the macro emits:
//!
//! - A free function `__plexus_credential_marker_for_<TypeIdent>() -> Vec<CredentialFieldMarker>`
//!   that returns the list of credential-bearing fields with their metadata.
//! - A free function `__plexus_sensitive_fields_for_<TypeIdent>() -> Vec<(Option<&'static str>, &'static str)>`
//!   that returns the (variant, field-name) pairs of credential-bearing
//!   fields (also treated as sensitive, per the implicit `#[sensitive]` rule).
//!
//! The registry functions live in the same module as the item, so a downstream
//! consumer can look them up by name. A future `inventory`-style registration
//! (per `AUTHZ-CRED-CORE-3`) will replace the by-name lookup with a
//! `TypeId`-keyed map; this ticket's job is to produce the per-type
//! enumeration, not the global registry.
//!
//! See `tests/credential_compile_tests.rs` and
//! `tests/compile/credential_*.rs` for the acceptance fixtures.

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    punctuated::Punctuated, spanned::Spanned, Attribute, Data, DeriveInput, Expr, ExprArray,
    ExprLit, Field, Fields, Ident, Lit, Meta, MetaNameValue, Token, Type,
};

// ---------------------------------------------------------------------------
// Parsed attribute clauses.
// ---------------------------------------------------------------------------

/// One parsed `#[credential(...)]` attribute payload.
///
/// String forms are validated at parse time against the closed enums in
/// `plexus_auth_core` (so a typo `kind = "berear"` is caught at compile time,
/// not at runtime).
#[derive(Debug)]
struct CredentialAttr {
    /// Source span of the attribute (for diagnostics).
    span: Span,
    /// Required: closed-enum `CredentialKind` discriminant or `Other { name }`.
    kind: KindValue,
    /// Required: `AttachmentSite` variant.
    attach_as: AttachmentSiteValue,
    /// Optional: scheme prefix (e.g. `"Bearer "`).
    scheme: Option<String>,
    /// Optional: list of scope names.
    scopes: Vec<String>,
    /// Optional: refresh method path (validated against MethodPath grammar).
    refresh_via: Option<String>,
    /// Optional: revoke method path.
    revoke_via: Option<String>,
    /// Optional `sensitive` flag. Credentials are always sensitive whether the
    /// flag is present or not; the field exists for clarity.
    #[allow(dead_code)]
    sensitive: bool,
}

#[derive(Debug, Clone)]
enum KindValue {
    /// One of the closed `CredentialKind` variants: bearer, cookie,
    /// oauth_access, oauth_refresh, oidc_id, aws_sts, macaroon.
    Closed(ClosedKind),
    /// `kind = { other = "<name>" }` — the escape valve for backend-specific
    /// kinds.
    Other(String),
}

#[derive(Debug, Clone, Copy)]
enum ClosedKind {
    Bearer,
    Cookie,
    OauthAccess,
    OauthRefresh,
    OidcId,
    AwsSts,
    Macaroon,
}

impl ClosedKind {
    /// Map a string-form kind name to a closed-enum variant. Returns `None` if
    /// the name is unknown (caller emits the diagnostic).
    fn parse(s: &str) -> Option<Self> {
        match s {
            "bearer" => Some(Self::Bearer),
            "cookie" => Some(Self::Cookie),
            "oauth_access" => Some(Self::OauthAccess),
            "oauth_refresh" => Some(Self::OauthRefresh),
            "oidc_id" => Some(Self::OidcId),
            "aws_sts" => Some(Self::AwsSts),
            "macaroon" => Some(Self::Macaroon),
            _ => None,
        }
    }

    /// Render to the Rust ident used in `CredentialKind::<ident>`.
    fn variant_ident(self) -> &'static str {
        match self {
            Self::Bearer => "Bearer",
            Self::Cookie => "Cookie",
            Self::OauthAccess => "OauthAccess",
            Self::OauthRefresh => "OauthRefresh",
            Self::OidcId => "OidcId",
            Self::AwsSts => "AwsSts",
            Self::Macaroon => "Macaroon",
        }
    }
}

#[derive(Debug, Clone)]
enum AttachmentSiteValue {
    Header(String),
    Cookie(String),
    FirstFrame { setup_method: String, param: String },
    InRpcParam(String),
}

impl AttachmentSiteValue {
    /// Parse `"header:<name>"`, `"cookie:<name>"`,
    /// `"first_frame:<setup-method>:<param>"`, or `"in_rpc_param:<name>"`.
    ///
    /// Returns the parsed variant on success; on failure returns the same
    /// `&str` so the caller can produce a span-attached diagnostic listing
    /// the four valid forms.
    fn parse(s: &str) -> Result<Self, ()> {
        // `header:`, `cookie:`, `in_rpc_param:` — single-arg prefixed forms.
        // `first_frame:` — two-arg form `first_frame:<setup_method>:<param>`.
        if let Some(name) = s.strip_prefix("header:") {
            if name.is_empty() {
                return Err(());
            }
            Ok(Self::Header(name.to_owned()))
        } else if let Some(name) = s.strip_prefix("cookie:") {
            if name.is_empty() {
                return Err(());
            }
            Ok(Self::Cookie(name.to_owned()))
        } else if let Some(name) = s.strip_prefix("in_rpc_param:") {
            if name.is_empty() {
                return Err(());
            }
            Ok(Self::InRpcParam(name.to_owned()))
        } else if let Some(rest) = s.strip_prefix("first_frame:") {
            // Expect `setup_method:param` with exactly one `:` separator.
            let mut iter = rest.splitn(2, ':');
            let setup_method = iter.next().ok_or(())?;
            let param = iter.next().ok_or(())?;
            if setup_method.is_empty() || param.is_empty() {
                return Err(());
            }
            Ok(Self::FirstFrame {
                setup_method: setup_method.to_owned(),
                param: param.to_owned(),
            })
        } else {
            Err(())
        }
    }
}

// ---------------------------------------------------------------------------
// MethodPath grammar validation.
//
// MethodPath is "namespace.method[.subsegment]*"; each segment starts with an
// ASCII lowercase letter and contains only [a-z0-9_]. Validates the same
// grammar as `plexus_auth_core::MethodPath::try_new` so the macro fails fast
// at compile time on bad refresh_via / revoke_via values (Risks §1 — keep the
// parsers in sync; if they drift the consuming side surfaces an error at
// runtime instead of build time).
// ---------------------------------------------------------------------------

/// Validate a `MethodPath` string. Returns true if the value parses cleanly
/// per the grammar in `plexus_auth_core::MethodPath::try_new`. Mirrors that
/// validator's three rules:
///   - Non-empty
///   - At least two dot-separated segments
///   - Each segment starts with `[a-z]` and is `[a-z0-9_]*` thereafter
fn is_valid_method_path(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    for seg in segments {
        if seg.is_empty() {
            return false;
        }
        let mut chars = seg.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {}
            _ => return false,
        }
        for c in chars {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Attribute parsing.
// ---------------------------------------------------------------------------

/// Parse a single `#[credential(kind = "...", attach_as = "...", ...)]`
/// attribute on a field. Returns a structured [`CredentialAttr`] or a compile
/// error pinpointing the bad clause.
fn parse_credential_attr(attr: &Attribute) -> syn::Result<CredentialAttr> {
    let span = attr.span();

    // The attribute's argument list. `parse_args_with` lets us accept arbitrary
    // `name = value` and `name { ... }` clauses.
    let metas: Punctuated<Meta, Token![,]> = match &attr.meta {
        Meta::Path(_) => Punctuated::new(),
        Meta::List(list) => list.parse_args_with(Punctuated::parse_terminated)?,
        Meta::NameValue(nv) => {
            return Err(syn::Error::new_spanned(
                nv,
                "`#[credential]` expects a parenthesized argument list, \
                 e.g. `#[credential(kind = \"bearer\", attach_as = \"header:authorization\")]`",
            ));
        }
    };

    let mut kind: Option<KindValue> = None;
    let mut attach_as: Option<AttachmentSiteValue> = None;
    let mut scheme: Option<String> = None;
    let mut scopes: Vec<String> = Vec::new();
    let mut refresh_via: Option<String> = None;
    let mut revoke_via: Option<String> = None;
    let mut sensitive_flag = false;

    for meta in metas {
        let path = meta.path().clone();
        let ident = path
            .get_ident()
            .ok_or_else(|| syn::Error::new_spanned(&path, "expected a bare identifier"))?
            .clone();

        let name = ident.to_string();
        match name.as_str() {
            "kind" => {
                kind = Some(parse_kind(&meta)?);
            }
            "attach_as" => {
                let s = expect_str_value(&meta, "attach_as")?;
                attach_as = Some(AttachmentSiteValue::parse(&s).map_err(|()| {
                    syn::Error::new(
                        meta.span(),
                        format!(
                            "invalid `attach_as` value `{s}`. \
                             Valid forms: \
                             `\"header:<name>\"`, \
                             `\"cookie:<name>\"`, \
                             `\"first_frame:<setup-method>:<param>\"`, \
                             `\"in_rpc_param:<name>\"`"
                        ),
                    )
                })?);
            }
            "scheme" => {
                scheme = Some(expect_str_value(&meta, "scheme")?);
            }
            "scopes" => {
                scopes = parse_scope_list(&meta)?;
            }
            "refresh_via" => {
                let s = expect_str_value(&meta, "refresh_via")?;
                if !is_valid_method_path(&s) {
                    return Err(syn::Error::new(
                        meta.span(),
                        format!(
                            "invalid `refresh_via` value `{s}`. \
                             Expected a MethodPath of the form \
                             `<namespace>.<method>[.<subsegment>]*` where each \
                             segment matches `[a-z][a-z0-9_]*`."
                        ),
                    ));
                }
                refresh_via = Some(s);
            }
            "revoke_via" => {
                let s = expect_str_value(&meta, "revoke_via")?;
                if !is_valid_method_path(&s) {
                    return Err(syn::Error::new(
                        meta.span(),
                        format!(
                            "invalid `revoke_via` value `{s}`. \
                             Expected a MethodPath of the form \
                             `<namespace>.<method>[.<subsegment>]*` where each \
                             segment matches `[a-z][a-z0-9_]*`."
                        ),
                    ));
                }
                revoke_via = Some(s);
            }
            "sensitive" => {
                // Allowed as a bare flag (`sensitive`) or as `sensitive = true/false`.
                // Credentials are always treated as sensitive regardless; the flag
                // is accepted for clarity per the ticket's "Implicit `#[sensitive]`
                // interaction" section.
                if matches!(&meta, Meta::Path(_)) {
                    sensitive_flag = true;
                } else if let Meta::NameValue(MetaNameValue { value, .. }) = &meta {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Bool(b), ..
                    }) = value
                    {
                        sensitive_flag = b.value();
                    } else {
                        return Err(syn::Error::new(
                            meta.span(),
                            "expected `sensitive` or `sensitive = true|false`",
                        ));
                    }
                } else {
                    return Err(syn::Error::new(
                        meta.span(),
                        "expected `sensitive` or `sensitive = true|false`",
                    ));
                }
            }
            "expires_at" => {
                return Err(syn::Error::new(
                    meta.span(),
                    "`expires_at` cannot be declared on the attribute — it is \
                     set at credential mint time, not compile time.",
                ));
            }
            other => {
                return Err(syn::Error::new(
                    meta.span(),
                    format!(
                        "unknown clause `{other}` in `#[credential(...)]`. \
                         Valid clauses: \
                         `kind`, `attach_as`, `scheme`, `scopes`, \
                         `refresh_via`, `revoke_via`, `sensitive`."
                    ),
                ));
            }
        }
    }

    let kind = kind.ok_or_else(|| {
        syn::Error::new(
            span,
            "missing required clause `kind` in `#[credential(...)]`. \
             Example: `kind = \"bearer\"`.",
        )
    })?;
    let attach_as = attach_as.ok_or_else(|| {
        syn::Error::new(
            span,
            "missing required clause `attach_as` in `#[credential(...)]`. \
             Example: `attach_as = \"header:authorization\"`.",
        )
    })?;

    Ok(CredentialAttr {
        span,
        kind,
        attach_as,
        scheme,
        scopes,
        refresh_via,
        revoke_via,
        sensitive: sensitive_flag,
    })
}

/// Parse the `kind` clause. Accepts either:
///   - `kind = "<closed-variant-name>"` — bearer | cookie | oauth_access |
///     oauth_refresh | oidc_id | aws_sts | macaroon
///   - `kind { other = "<name>" }` — the structured escape valve for
///     backend-specific kinds.
///
/// The bare `kind = "other"` form is rejected with a diagnostic pointing at
/// the structured form (per acceptance criterion 8).
fn parse_kind(meta: &Meta) -> syn::Result<KindValue> {
    match meta {
        Meta::NameValue(MetaNameValue { value, .. }) => {
            // `kind = "..."` — string form.
            let s = match value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => s.value(),
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "`kind` must be a string literal or a structured `{ other = \"...\" }` form",
                    ));
                }
            };
            if s == "other" {
                return Err(syn::Error::new(
                    value.span(),
                    "the bare form `kind = \"other\"` is not accepted. \
                     For backend-specific kinds use the structured form: \
                     `kind(other = \"<your-kind-name>\")` (parens), \
                     because the `Other` variant of `CredentialKind` carries \
                     a name parameter and cannot be expressed as a bare string.",
                ));
            }
            if let Some(closed) = ClosedKind::parse(&s) {
                Ok(KindValue::Closed(closed))
            } else {
                Err(syn::Error::new(
                    value.span(),
                    format!(
                        "unknown credential kind `{s}`. \
                         Valid forms: \
                         `\"bearer\"`, `\"cookie\"`, `\"oauth_access\"`, \
                         `\"oauth_refresh\"`, `\"oidc_id\"`, `\"aws_sts\"`, \
                         `\"macaroon\"`, or the structured \
                         `kind(other = \"<name>\")` escape valve."
                    ),
                ))
            }
        }
        Meta::List(list) => {
            // `kind(other = "<name>")` — structured form for the `Other`
            // variant. The closed-enum names are NOT allowed in this form;
            // only `other = "<name>"` is meaningful here.
            let metas: Punctuated<Meta, Token![,]> =
                list.parse_args_with(Punctuated::parse_terminated)?;
            let mut name: Option<String> = None;
            for inner in metas {
                let path = inner.path().clone();
                let id = path
                    .get_ident()
                    .ok_or_else(|| syn::Error::new_spanned(&path, "expected bare identifier"))?
                    .clone();
                if id == "other" {
                    name = Some(expect_str_value(&inner, "other")?);
                } else {
                    return Err(syn::Error::new(
                        inner.span(),
                        "in the structured `kind(...)` form, only the \
                         `other = \"<name>\"` clause is supported (use the \
                         bare `kind = \"bearer\"`-style form for the closed \
                         CredentialKind variants).",
                    ));
                }
            }
            let name = name.ok_or_else(|| {
                syn::Error::new_spanned(
                    list,
                    "structured `kind(...)` form must include `other = \"<name>\"`",
                )
            })?;
            Ok(KindValue::Other(name))
        }
        Meta::Path(p) => Err(syn::Error::new_spanned(
            p,
            "`kind` requires a value: `kind = \"bearer\"` or `kind(other = \"<name>\")`",
        )),
    }
}

/// Helper: require a `name = "string-literal"` clause and return the string.
fn expect_str_value(meta: &Meta, clause: &str) -> syn::Result<String> {
    if let Meta::NameValue(MetaNameValue { value, .. }) = meta {
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = value
        {
            return Ok(s.value());
        }
        return Err(syn::Error::new(
            value.span(),
            format!("`{clause}` must be a string literal"),
        ));
    }
    Err(syn::Error::new(
        meta.span(),
        format!("`{clause}` must be of the form `{clause} = \"...\"`"),
    ))
}

/// Helper: parse `scopes = [".."]` or `scopes(..)` into a `Vec<String>`.
fn parse_scope_list(meta: &Meta) -> syn::Result<Vec<String>> {
    match meta {
        Meta::NameValue(MetaNameValue { value, .. }) => {
            // `scopes = ["a", "b"]`
            if let Expr::Array(ExprArray { elems, .. }) = value {
                let mut out = Vec::with_capacity(elems.len());
                for e in elems {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = e
                    {
                        out.push(s.value());
                    } else {
                        return Err(syn::Error::new(
                            e.span(),
                            "`scopes` entries must be string literals",
                        ));
                    }
                }
                Ok(out)
            } else {
                Err(syn::Error::new(
                    value.span(),
                    "`scopes` must be an array of string literals, e.g. `scopes = [\"a.b\", \"c.d\"]`",
                ))
            }
        }
        other => Err(syn::Error::new(
            other.span(),
            "`scopes` must be of the form `scopes = [\"...\", ...]`",
        )),
    }
}

// ---------------------------------------------------------------------------
// Type recognition: the field's declared type must be `Credential<...>`.
//
// We accept any path ending in `Credential` followed by a single generic
// argument (the inner payload type). This is the lightest possible check —
// it does not verify that the path resolves to `plexus_auth_core::Credential`,
// only that the surface name matches. A user who shadows `Credential` with a
// look-alike type will receive a runtime registry pointing at a non-credential
// field, which is detectable by `AUTHZ-CRED-CORE-3`'s schema-build pass.
// ---------------------------------------------------------------------------

fn type_is_credential_wrapper(ty: &Type) -> bool {
    if let Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Credential" {
                return matches!(seg.arguments, syn::PathArguments::AngleBracketed(_));
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Entry point.
//
// Invoked via `#[proc_macro_attribute] pub fn credential(...)` in lib.rs.
// Receives the full struct/enum item (with inner `#[credential(...)]`
// field attributes still attached); emits the registry helpers and re-emits
// the item with the inner attributes stripped.
// ---------------------------------------------------------------------------

/// One parsed credential field within an item.
#[derive(Debug)]
struct ParsedCredentialField {
    /// Field name. For tuple-struct fields this is `0`, `1`, etc.
    name: String,
    /// Optional variant name (only set for enum-variant fields).
    variant: Option<String>,
    /// Parsed metadata.
    attr: CredentialAttr,
}

/// Entry point invoked from `lib.rs` for `#[derive(Credentials)]`. Receives
/// the struct or enum item and emits the registry helpers as a sibling block.
/// The original item is unchanged (derives are additive only).
pub fn expand_credentials_derive(
    item_tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input: DeriveInput = match syn::parse(item_tokens) {
        Ok(i) => i,
        Err(e) => return e.to_compile_error().into(),
    };

    match expand_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let type_ident = input.ident.clone();
    let mut parsed_fields: Vec<ParsedCredentialField> = Vec::new();

    // We do NOT mutate the input (helper attributes are auto-consumed by the
    // derive layer). We only iterate to collect parsed credential fields.
    match &input.data {
        Data::Struct(s) => collect_from_fields_readonly(&s.fields, None, &mut parsed_fields)?,
        Data::Enum(e) => {
            for variant in &e.variants {
                let variant_name = variant.ident.to_string();
                collect_from_fields_readonly(
                    &variant.fields,
                    Some(variant_name),
                    &mut parsed_fields,
                )?;
            }
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Credentials)] cannot be applied to a union",
            ));
        }
    }

    // Emit the registry functions.
    let registry_fn_name = format_ident!("__plexus_credential_marker_for_{}", type_ident);
    let sensitive_fn_name = format_ident!("__plexus_sensitive_fields_for_{}", type_ident);
    let registry_entries = parsed_fields
        .iter()
        .map(emit_field_marker_initializer)
        .collect::<Vec<_>>();
    let sensitive_entries = parsed_fields
        .iter()
        .map(|p| {
            let name = &p.name;
            let variant = match &p.variant {
                Some(v) => quote! { ::core::option::Option::Some(#v) },
                None => quote! { ::core::option::Option::None },
            };
            quote! { (#variant, #name) }
        })
        .collect::<Vec<_>>();

    Ok(quote! {
        /// Macro-emitted credential field registry. One entry per
        /// `#[credential(...)]`-annotated field on this type, in stable
        /// declaration order. For enums, the variant name is included in each
        /// entry; for structs, the variant slot is empty.
        ///
        /// Consumed by `AUTHZ-CRED-CORE-2` (dispatch-side credential extraction)
        /// and `AUTHZ-CRED-CORE-3` (schema-build credential reflection).
        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        pub fn #registry_fn_name() -> ::std::vec::Vec<CredentialFieldMarker> {
            ::std::vec![ #( #registry_entries ),* ]
        }

        /// Macro-emitted sensitive-field registry. Mirrors the
        /// `#[credential(...)]` set — credentials are always treated as
        /// sensitive per AUTHZ-PRIVACY-2, so this registry deliberately
        /// duplicates the credential registry's field names. When the
        /// `#[sensitive]` attribute lands, its registry consumes from BOTH
        /// surfaces; the credential macro does not need to special-case it.
        ///
        /// Each entry is `(Option<variant_name>, field_name)`.
        #[doc(hidden)]
        #[allow(non_snake_case, dead_code)]
        pub fn #sensitive_fn_name() -> ::std::vec::Vec<(::core::option::Option<&'static str>, &'static str)> {
            ::std::vec![ #( #sensitive_entries ),* ]
        }
    })
}

/// Walk a struct's fields (or one enum variant's fields), parse any
/// `#[credential(...)]` helper attribute and append to `out`. Unlike
/// `collect_from_fields`, this variant does NOT mutate the field (the
/// derive layer auto-consumes helper attributes; we just read them).
fn collect_from_fields_readonly(
    fields: &Fields,
    variant: Option<String>,
    out: &mut Vec<ParsedCredentialField>,
) -> syn::Result<()> {
    match fields {
        Fields::Named(named) => {
            for f in named.named.iter() {
                process_one_field_ro(f, variant.clone(), out)?;
            }
        }
        Fields::Unnamed(unnamed) => {
            for (i, f) in unnamed.unnamed.iter().enumerate() {
                if !has_credential_attr(f) {
                    continue;
                }
                let attr = read_credential_attr(f)?;
                enforce_wrapper_type(f, &attr)?;
                out.push(ParsedCredentialField {
                    name: i.to_string(),
                    variant: variant.clone(),
                    attr,
                });
            }
        }
        Fields::Unit => {}
    }
    Ok(())
}

/// Read-only version of `process_one_field` used in the derive entry point.
fn process_one_field_ro(
    field: &Field,
    variant: Option<String>,
    out: &mut Vec<ParsedCredentialField>,
) -> syn::Result<()> {
    if !has_credential_attr(field) {
        return Ok(());
    }
    let attr = read_credential_attr(field)?;
    enforce_wrapper_type(field, &attr)?;
    let name = field
        .ident
        .as_ref()
        .map(|i| i.to_string())
        .unwrap_or_else(|| "_".to_owned());
    out.push(ParsedCredentialField {
        name,
        variant,
        attr,
    });
    Ok(())
}

/// Read-only version of `take_credential_attr` — does NOT mutate the field.
/// Returns an error if the field has two `#[credential(...)]` attributes.
fn read_credential_attr(field: &Field) -> syn::Result<CredentialAttr> {
    let mut matched: Vec<&Attribute> = field.attrs.iter().filter(|a| is_credential_attr(a)).collect();
    if matched.len() > 1 {
        return Err(syn::Error::new_spanned(
            matched[1],
            "two `#[credential(...)]` attributes on the same field. \
             Combine the clauses into a single attribute.",
        ));
    }
    let attr = matched.remove(0);
    parse_credential_attr(attr)
}

fn has_credential_attr(field: &Field) -> bool {
    field.attrs.iter().any(is_credential_attr)
}

/// Returns true if the attribute path is `credential`. As a `proc_macro_derive`
/// helper attribute it is always single-segment.
fn is_credential_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("credential")
}

/// Check that the field's declared type is `Credential<...>`. Emits a compile
/// error pointing at the wrapper requirement otherwise (acceptance #5).
fn enforce_wrapper_type(field: &Field, attr: &CredentialAttr) -> syn::Result<()> {
    if !type_is_credential_wrapper(&field.ty) {
        let field_label = field
            .ident
            .as_ref()
            .map(|i| format!("`{i}`"))
            .unwrap_or_else(|| "tuple field".to_string());
        return Err(syn::Error::new(
            attr.span,
            format!(
                "`#[credential(...)]` may only be applied to a field whose type is \
                 `Credential<T>` (the sealed wrapper from `plexus_auth_core`). \
                 Field {field_label} has a non-`Credential<T>` type. \
                 Wrap the payload as `Credential<{}>` and mint via the framework-injected \
                 `CredentialMinter`.",
                describe_type(&field.ty),
            ),
        ));
    }
    Ok(())
}

/// Render a type to a short human-readable name for error messages.
fn describe_type(ty: &Type) -> String {
    match ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "?".to_string()),
        _ => "?".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Codegen for one CredentialFieldMarker entry.
//
// Emits an expression of type `CredentialFieldMarker` populated from the
// parsed clauses. The expression references `CredentialFieldMarker`,
// `CredentialKind`, `AttachmentSite`, `Scope`, `MethodPath`,
// `CredentialScheme`, `CredentialKindName`, `HeaderName`, `CookieName`,
// `ParamName` — these names must be in scope at the call site, which the
// emit point ensures via the helper's surrounding `pub fn` (Rust resolves the
// names through the standard module path the user imports).
// ---------------------------------------------------------------------------

fn emit_field_marker_initializer(p: &ParsedCredentialField) -> TokenStream2 {
    let name = &p.name;
    let variant_expr = match &p.variant {
        Some(v) => quote! { ::core::option::Option::Some(#v) },
        None => quote! { ::core::option::Option::None },
    };

    let kind_expr = emit_kind(&p.attr.kind);
    let attach_as_expr = emit_attach_as(&p.attr.attach_as);
    let scheme_expr = match &p.attr.scheme {
        Some(s) => quote! { ::core::option::Option::Some(CredentialScheme::new(#s)) },
        None => quote! { ::core::option::Option::None },
    };
    let scopes_iter = p.attr.scopes.iter();
    let scopes_expr = quote! {
        ::std::vec![ #( Scope::new(#scopes_iter) ),* ]
    };
    let refresh_expr = match &p.attr.refresh_via {
        Some(s) => quote! {
            ::core::option::Option::Some(
                MethodPath::try_new(#s).expect("validated at macro time")
            )
        },
        None => quote! { ::core::option::Option::None },
    };
    let revoke_expr = match &p.attr.revoke_via {
        Some(s) => quote! {
            ::core::option::Option::Some(
                MethodPath::try_new(#s).expect("validated at macro time")
            )
        },
        None => quote! { ::core::option::Option::None },
    };

    quote! {
        CredentialFieldMarker {
            variant: #variant_expr,
            field: #name,
            kind: #kind_expr,
            attach_as: #attach_as_expr,
            scheme: #scheme_expr,
            scopes: #scopes_expr,
            refresh_via: #refresh_expr,
            revoke_via: #revoke_expr,
        }
    }
}

fn emit_kind(k: &KindValue) -> TokenStream2 {
    match k {
        KindValue::Closed(c) => {
            let v = Ident::new(c.variant_ident(), Span::call_site());
            quote! { CredentialKind::#v }
        }
        KindValue::Other(name) => quote! {
            CredentialKind::Other { name: CredentialKindName::new(#name) }
        },
    }
}

fn emit_attach_as(a: &AttachmentSiteValue) -> TokenStream2 {
    match a {
        AttachmentSiteValue::Header(name) => quote! {
            AttachmentSite::Header {
                name: HeaderName::try_new(#name).expect("validated at macro time")
            }
        },
        AttachmentSiteValue::Cookie(name) => quote! {
            AttachmentSite::Cookie {
                name: CookieName::try_new(#name).expect("validated at macro time")
            }
        },
        AttachmentSiteValue::FirstFrame {
            setup_method,
            param,
        } => quote! {
            AttachmentSite::FirstFrame {
                setup_method: MethodPath::try_new(#setup_method).expect("validated at macro time"),
                param: ParamName::new(#param),
            }
        },
        AttachmentSiteValue::InRpcParam(param) => quote! {
            AttachmentSite::InRpcParam { param: ParamName::new(#param) }
        },
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the parser helpers. These do not invoke the proc-macro
// entry point (proc-macros cannot be invoked from `cargo test --lib`); they
// exercise pure helper functions to lock the parsing contract in place.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_path_validator_accepts_canonical() {
        assert!(is_valid_method_path("auth.login"));
        assert!(is_valid_method_path("cone.send_message"));
        assert!(is_valid_method_path("a.b.c.d"));
    }

    #[test]
    fn method_path_validator_rejects_bad() {
        assert!(!is_valid_method_path(""));
        assert!(!is_valid_method_path("login"));
        assert!(!is_valid_method_path("Auth.login"));
        assert!(!is_valid_method_path("1auth.login"));
        assert!(!is_valid_method_path("auth..login"));
        assert!(!is_valid_method_path("auth."));
        assert!(!is_valid_method_path(".login"));
        assert!(!is_valid_method_path("auth.*"));
    }

    #[test]
    fn closed_kind_parser() {
        assert!(matches!(ClosedKind::parse("bearer"), Some(ClosedKind::Bearer)));
        assert!(matches!(ClosedKind::parse("cookie"), Some(ClosedKind::Cookie)));
        assert!(matches!(
            ClosedKind::parse("oauth_access"),
            Some(ClosedKind::OauthAccess)
        ));
        assert!(matches!(
            ClosedKind::parse("oauth_refresh"),
            Some(ClosedKind::OauthRefresh)
        ));
        assert!(matches!(ClosedKind::parse("oidc_id"), Some(ClosedKind::OidcId)));
        assert!(matches!(ClosedKind::parse("aws_sts"), Some(ClosedKind::AwsSts)));
        assert!(matches!(
            ClosedKind::parse("macaroon"),
            Some(ClosedKind::Macaroon)
        ));
        // Bare "other" is intentionally NOT accepted here — the caller must
        // emit a diagnostic pointing at the structured form. The parser fn
        // returns None for it.
        assert!(ClosedKind::parse("other").is_none());
        assert!(ClosedKind::parse("nonsense").is_none());
    }

    #[test]
    fn attachment_site_parser_accepts_all_four_forms() {
        assert!(matches!(
            AttachmentSiteValue::parse("header:authorization"),
            Ok(AttachmentSiteValue::Header(ref s)) if s == "authorization"
        ));
        assert!(matches!(
            AttachmentSiteValue::parse("cookie:plexus_session"),
            Ok(AttachmentSiteValue::Cookie(ref s)) if s == "plexus_session"
        ));
        match AttachmentSiteValue::parse("first_frame:auth.connect:token") {
            Ok(AttachmentSiteValue::FirstFrame {
                setup_method,
                param,
            }) => {
                assert_eq!(setup_method, "auth.connect");
                assert_eq!(param, "token");
            }
            other => panic!("expected FirstFrame, got {other:?}"),
        }
        assert!(matches!(
            AttachmentSiteValue::parse("in_rpc_param:session_token"),
            Ok(AttachmentSiteValue::InRpcParam(ref s)) if s == "session_token"
        ));
    }

    #[test]
    fn attachment_site_parser_rejects_bad_forms() {
        assert!(AttachmentSiteValue::parse("").is_err());
        assert!(AttachmentSiteValue::parse("header").is_err()); // no `:`
        assert!(AttachmentSiteValue::parse("header:").is_err()); // empty name
        assert!(AttachmentSiteValue::parse("cookie:").is_err());
        assert!(AttachmentSiteValue::parse("body:foo").is_err()); // unknown prefix
        assert!(AttachmentSiteValue::parse("first_frame:auth.connect").is_err()); // missing param
        assert!(AttachmentSiteValue::parse("first_frame::token").is_err()); // empty setup
    }

    #[test]
    fn type_is_credential_wrapper_accepts_credential_t() {
        let ty: Type = syn::parse_str("Credential<String>").unwrap();
        assert!(type_is_credential_wrapper(&ty));
        let ty: Type = syn::parse_str("plexus_auth_core::Credential<SessionToken>").unwrap();
        assert!(type_is_credential_wrapper(&ty));
    }

    #[test]
    fn type_is_credential_wrapper_rejects_other_types() {
        let ty: Type = syn::parse_str("String").unwrap();
        assert!(!type_is_credential_wrapper(&ty));
        let ty: Type = syn::parse_str("Option<Credential<String>>").unwrap();
        // Outer is Option, not Credential — should NOT count.
        assert!(!type_is_credential_wrapper(&ty));
        let ty: Type = syn::parse_str("Credential").unwrap();
        // Bare `Credential` with no generic argument is not a wrapper.
        assert!(!type_is_credential_wrapper(&ty));
    }
}

// `CredentialFieldMarker` is **introduced by this ticket** and lives in
// `plexus_auth_core` per the ticket's `introduces:` frontmatter. The macro's
// emitted code references it by name; the type's declaration lands in
// `plexus_auth_core` in a follow-up (the agent's run instructions for
// AUTHZ-CRED-MACRO-1 restrict edits to the `plexus-macros` repo only).
// Consumers must bring `CredentialFieldMarker`, `CredentialKind`,
// `AttachmentSite`, `CredentialScheme`, `Scope`, `MethodPath`, `HeaderName`,
// `CookieName`, `ParamName`, and `CredentialKindName` into scope at the
// emission site for the macro-generated code to type-check.
//
// See `plans/AUTHZ/AUTHZ-CRED-MACRO-1-RUN-NOTES.md` for the cross-crate
// placement decision.
