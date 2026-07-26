//! The token-level mirror of the IR — everything a proc macro can actually
//! decide, and nothing it cannot.
//!
//! Each type here corresponds 1:1 to a `plexus_core::ir` type, with exactly one
//! difference: wherever the IR holds a `SchemaRef` (a *resolved* type) this
//! holds a [`syn::Type`] (the type's **tokens**). Nothing is stringly typed —
//! the failure mode PLX-75 designed out was storing a type as a `String` and
//! re-parsing it later, so a type that reaches emission does so as the very
//! tokens the author wrote.
//!
//! `HttpMethodIrSpec` / `AuthRequirementSpec` are token-level mirrors of
//! `plexus_core::ir::{HttpMethodIr, AuthRequirementIr}` for the same reason a
//! `syn::Type` is not a `plexus_core` type: this crate cannot depend on
//! plexus-core (plexus-core depends on *it* — a normal dependency would be a
//! cycle). They are real enums, mapped to the real enum's variants by an
//! exhaustive match in [`emit`](super::emit) with no `String` in between and no
//! "should not happen" arm.

use syn::{ItemImpl, TraitItemFn, Type};

use super::attrs::MethodAttrs;
use super::signature::{self, ReturnShape};
use crate::parse::{extract_doc_description, parse_deprecation_attrs, ParsedDeprecation};

/// Token-level mirror of `plexus_core::ir::HttpMethodIr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HttpMethodIrSpec {
    Get,
    #[default]
    Post,
    Put,
    Delete,
    Patch,
}

/// Token-level mirror of `plexus_core::ir::AuthRequirementIr`.
///
/// Derived from the *signature*, not from an attribute: `auth: &AuthContext`
/// means the method demands a caller, `auth: Option<&AuthContext>` means it
/// accepts one. Absent an auth parameter the IR's default-deny `Required`
/// stands (AUTHZ-CORE-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AuthRequirementSpec {
    #[allow(dead_code)] // reachable only once build F wires a `public` source
    Public,
    Optional,
    #[default]
    Required,
}

/// Token-level mirror of `plexus_core::ir::ParamIr`.
#[derive(Debug, Clone)]
pub(crate) struct ParamIrSpec {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The parameter's type, as tokens. For `Option<T>` this is the **inner**
    /// `T`: the IR spells optionality with `required: false`, not by wrapping
    /// the schema.
    pub(crate) ty: Type,
    pub(crate) required: bool,
}

/// Token-level mirror of `plexus_core::ir::MethodIr`.
#[derive(Debug, Clone)]
pub(crate) struct MethodIrSpec {
    pub(crate) name: String,
    pub(crate) dotted_id: String,
    pub(crate) description: String,
    pub(crate) params: Vec<ParamIrSpec>,
    /// Item type of the update stream, or `None` for a turn that emits none.
    pub(crate) updates: Option<Type>,
    /// Type of the terminal value, or `None` when the turn resolves to nothing
    /// meaningful. Never `()` — the unit case *is* the `None` case, which is
    /// what preserves PLX-75's "a `SchemaRef` can never denote `()`" property
    /// through the parser (see [`signature::classify_return`]).
    pub(crate) terminal: Option<Type>,
    /// The capability set `C` from the method's `Client<C>` handle, as tokens.
    /// `None` when the method takes no handle — an empty callback set.
    pub(crate) capability_set: Option<Type>,
    pub(crate) http_method: HttpMethodIrSpec,
    pub(crate) auth: AuthRequirementSpec,
    pub(crate) deprecation: Option<ParsedDeprecation>,
    /// PLX-71 `ext(...)` pairs, parsed by the existing parser and passed
    /// through verbatim.
    pub(crate) extensions: Vec<(String, serde_json::Value)>,
}

impl MethodIrSpec {
    /// Parse one `#[method(...)]`-annotated signature.
    ///
    /// `namespace` is the dotted path of the owning activation; `dotted_id`
    /// becomes `"{namespace}.{name}"` explicitly rather than being left for a
    /// consumer to reconstruct (PLX-73 `q-ir-completeness` item 6).
    pub(crate) fn from_signature(namespace: &str, func: &TraitItemFn) -> syn::Result<Self> {
        let attrs = MethodAttrs::parse_from(&func.attrs)?;

        let name = attrs
            .name
            .clone()
            .unwrap_or_else(|| func.sig.ident.to_string());
        let dotted_id = format!("{namespace}.{name}");

        let description = attrs
            .description
            .clone()
            .or_else(|| extract_doc_description(&func.attrs))
            .unwrap_or_default();

        let signature::Signature {
            params,
            capability_set,
            auth,
        } = signature::parse_inputs(&func.sig, &attrs.param_docs)?;

        let ReturnShape { updates, terminal } = signature::classify_return(&func.sig.output);

        Ok(Self {
            name,
            dotted_id,
            description,
            params,
            updates,
            terminal,
            capability_set,
            http_method: attrs.http_method.unwrap_or_default(),
            auth,
            deprecation: parse_deprecation_attrs(&func.attrs, func.sig.ident.span())?,
            extensions: attrs.extensions,
        })
    }
}

/// Token-level mirror of `plexus_core::ir::ChildEdge`.
///
/// A child's subtree is an `ActivationIr` the parser cannot see: it belongs to
/// another type, in another impl block, possibly another crate. So a `Static` /
/// `Indexed` edge carries the child **type's tokens**, and emission asks that
/// type for its own IR through the crate-internal
/// `__plexus_activation_ir()` convention (see [`emit`](super::emit)).
#[derive(Debug, Clone)]
pub(crate) enum ChildEdgeSpec {
    /// `#[child] fn ns(&self) -> ChildTy` — the whole subtree is embedded.
    Static {
        namespace: String,
        child_ty: Type,
    },
    /// `#[child] fn ns(&self, name: &str) -> Option<ChildTy>` — runtime
    /// registered, deliberately not embedded.
    Dynamic {
        namespace: String,
        description: String,
    },
    /// `#[child(list = "...")] fn ns(&self) -> ChildTy` — an indexed family.
    Indexed {
        namespace: String,
        list_method: String,
        search_method: Option<String>,
        id_field: String,
        path_template: String,
        child_ty: Type,
        description: String,
    },
}

/// Token-level mirror of `plexus_core::ir::ActivationIr`.
#[derive(Debug, Clone)]
pub(crate) struct ActivationIrSpec {
    pub(crate) namespace: String,
    pub(crate) version: String,
    pub(crate) backend_name: Option<String>,
    pub(crate) respond_method: Option<String>,
    pub(crate) description: String,
    pub(crate) long_description: Option<String>,
    pub(crate) methods: Vec<MethodIrSpec>,
    pub(crate) children: Vec<ChildEdgeSpec>,
    pub(crate) deprecation: Option<ParsedDeprecation>,
}

impl ActivationIrSpec {
    /// Assemble a whole activation: every `#[method]` in the impl block, plus a
    /// child edge per `#[child]`.
    pub(crate) fn from_impl(
        namespace: &str,
        version: &str,
        backend_name: Option<String>,
        respond_method: Option<String>,
        description: Option<String>,
        item: &ItemImpl,
    ) -> syn::Result<Self> {
        let mut methods = Vec::new();
        let mut children = Vec::new();

        for it in &item.items {
            let syn::ImplItem::Fn(f) = it else { continue };

            if crate::parse::has_child_attr(f) {
                children.push(signature::child_edge(namespace, f)?);
                continue;
            }
            if !crate::parse::has_method_attr(f) {
                continue;
            }

            // `TraitItemFn` and `ImplItemFn` differ only in the body's
            // representation; the parser reads attributes and the signature.
            let as_trait_fn = TraitItemFn {
                attrs: f.attrs.clone(),
                sig: f.sig.clone(),
                default: None,
                semi_token: Some(Default::default()),
            };
            methods.push(MethodIrSpec::from_signature(namespace, &as_trait_fn)?);
        }

        let doc = extract_doc_description(&item.attrs);
        let (description, long_description) = match (description, doc) {
            (Some(d), doc) => (d, doc),
            (None, Some(doc)) => match doc.split_once('\n') {
                Some((first, rest)) => (first.to_string(), Some(rest.trim().to_string())),
                None => (doc, None),
            },
            (None, None) => (String::new(), None),
        };

        Ok(Self {
            namespace: namespace.to_string(),
            version: version.to_string(),
            backend_name,
            respond_method,
            description,
            long_description,
            methods,
            children,
            deprecation: parse_deprecation_attrs(&item.attrs, item.impl_token.span)?,
        })
    }
}
