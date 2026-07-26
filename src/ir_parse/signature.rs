//! Signature reading: parameters, the capability handle, and the turn envelope.
//!
//! Everything in this file is a decision about **tokens**. No type is resolved,
//! no schema is computed and no capability is looked up in a table — the third
//! of those is the load-bearing one: the capability set is carried out of here
//! as the author's own `syn::Type` and handed back to rustc at emission, so
//! `plexus-core`'s sealed marker set stays the only registry that exists.

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{
    FnArg, GenericArgument, ImplItemFn, PatType, PathArguments, ReturnType, Type, TypeParamBound,
};

use super::spec::{AuthRequirementSpec, ChildEdgeSpec, ParamIrSpec};
use crate::parse::{AuthParamShape, ChildMethodInfo, ChildMethodKind};

/// Parameter attributes that mean "not a caller-supplied parameter": the value
/// is injected by the framework, so it must not appear in `MethodIr::params`.
const INJECTED_PARAM_ATTRS: &[&str] = &["from_auth", "activation_param"];

/// What [`parse_inputs`] extracted from a signature's argument list.
pub(crate) struct Signature {
    pub(crate) params: Vec<ParamIrSpec>,
    pub(crate) capability_set: Option<Type>,
    pub(crate) auth: AuthRequirementSpec,
}

/// Split a signature's inputs into declared parameters, the capability handle
/// and the auth requirement.
///
/// The `Client<C>` handle is **removed** from the parameter list: it is not
/// something a caller sends, it is the callback declaration. That is the same
/// treatment the `auth:` parameter gets, and for the same reason.
pub(crate) fn parse_inputs(
    sig: &syn::Signature,
    param_docs: &std::collections::HashMap<String, String>,
) -> syn::Result<Signature> {
    let mut params = Vec::new();
    let mut capability_set: Option<Type> = None;
    let mut capability_span: Option<proc_macro2::Span> = None;
    let mut auth = AuthRequirementSpec::default();

    for arg in &sig.inputs {
        let FnArg::Typed(pt) = arg else { continue }; // `&self`

        // Framework-injected parameters never reach the wire.
        if pt.attrs.iter().any(|a| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| INJECTED_PARAM_ATTRS.contains(&s.ident.to_string().as_str()))
        }) {
            continue;
        }

        // The auth parameter sets `MethodIr::auth`, and is not a parameter.
        if let Some(shape) = crate::parse::classify_auth_context_type(&pt.ty) {
            auth = match shape {
                AuthParamShape::Required => AuthRequirementSpec::Required,
                AuthParamShape::Optional => AuthRequirementSpec::Optional,
            };
            continue;
        }

        // The capability handle declares the callbacks, and is not a parameter.
        if let Some(set) = capability_handle_set(&pt.ty)? {
            if capability_span.is_some() {
                return Err(syn::Error::new(
                    pt.ty.span(),
                    "a method may take at most one capability handle; merge the \
                     capability sets into one `Client<(..)>`",
                ));
            }
            capability_span = Some(pt.ty.span());
            capability_set = Some(set);
            continue;
        }

        params.push(param_spec(pt, param_docs)?);
    }

    Ok(Signature {
        params,
        capability_set,
        auth,
    })
}

/// One declared parameter. `Option<T>` is spelled `required: false` over the
/// inner `T`, matching `ParamIr`'s contract.
fn param_spec(
    pt: &PatType,
    param_docs: &std::collections::HashMap<String, String>,
) -> syn::Result<ParamIrSpec> {
    let syn::Pat::Ident(ident) = &*pt.pat else {
        return Err(syn::Error::new(
            pt.pat.span(),
            "method parameters must be simple identifiers, e.g. `path: String`",
        ));
    };
    let name = ident.ident.to_string();

    let (ty, required) = match option_inner(&pt.ty) {
        Some(inner) => (inner, false),
        None => ((*pt.ty).clone(), true),
    };

    if is_unit(&ty) {
        return Err(syn::Error::new(
            pt.ty.span(),
            format!(
                "parameter `{name}` has the unit type; a parameter that carries no \
                 data is not representable in the IR — remove it"
            ),
        ));
    }

    Ok(ParamIrSpec {
        description: param_docs.get(&name).cloned().unwrap_or_default(),
        name,
        ty,
        required,
    })
}

/// The turn envelope, derived from the return type.
pub(crate) struct ReturnShape {
    pub(crate) updates: Option<Type>,
    pub(crate) terminal: Option<Type>,
}

/// Map a return type onto `(updates, terminal)`.
///
/// | signature | updates | terminal |
/// |---|---|---|
/// | *(none)* / `-> ()` | `None` | `None` |
/// | `-> Result<T, E>` | `None` | `T` (`None` if `T` is `()`) |
/// | `-> T` | `None` | `T` |
/// | `-> impl Stream<Item = T>` | `T` | `None` |
/// | `-> Result<impl Stream<Item = T>, E>` | `T` | `None` |
///
/// The streaming rows are where PLX-75's no-unit-`SchemaRef` property is
/// preserved: a streaming method has no *distinct* terminal type, so `terminal`
/// is `None`. Nothing here ever produces a `SchemaRef` for `()` — a unit type
/// is discarded at the token level, before emission can see it, which is why no
/// code path in this crate can construct one.
pub(crate) fn classify_return(output: &ReturnType) -> ReturnShape {
    let ReturnType::Type(_, ty) = output else {
        return ReturnShape {
            updates: None,
            terminal: None,
        };
    };

    if let Some(item) = stream_item(ty) {
        return ReturnShape {
            updates: non_unit(item),
            terminal: None,
        };
    }

    if let Some(ok_ty) = result_ok(ty) {
        if let Some(item) = stream_item(&ok_ty) {
            return ReturnShape {
                updates: non_unit(item),
                terminal: None,
            };
        }
        return ReturnShape {
            updates: None,
            terminal: non_unit(ok_ty),
        };
    }

    ReturnShape {
        updates: None,
        terminal: non_unit((**ty).clone()),
    }
}

/// Build a [`ChildEdgeSpec`] from a `#[child]`-annotated method.
///
/// Reuses [`ChildMethodInfo::from_fn`] for signature validation and the
/// `list`/`search` arguments, so there is one implementation of the `#[child]`
/// grammar.
pub(crate) fn child_edge(namespace: &str, f: &ImplItemFn) -> syn::Result<ChildEdgeSpec> {
    let info = ChildMethodInfo::from_fn(f)?;
    let child_namespace = info.fn_name.to_string();
    let description = info.description.clone();

    match info.kind {
        ChildMethodKind::Dynamic => Ok(ChildEdgeSpec::Dynamic {
            namespace: child_namespace,
            description,
        }),
        ChildMethodKind::Static => {
            let ReturnType::Type(_, ret) = &f.sig.output else {
                return Err(syn::Error::new(
                    f.sig.span(),
                    "a #[child] method must declare a return type",
                ));
            };
            let child_ty = (**ret).clone();

            match &info.list_fn {
                None => Ok(ChildEdgeSpec::Static {
                    namespace: child_namespace,
                    child_ty,
                }),
                Some(list_fn) => Ok(ChildEdgeSpec::Indexed {
                    list_method: format!("{namespace}.{list_fn}"),
                    search_method: info
                        .search_fn
                        .as_ref()
                        .map(|s| format!("{namespace}.{s}")),
                    // PLX-73 `q-ir-completeness` item 17: the old client
                    // *guessed* this field with a `"name"` fallback. Declaring
                    // the guess is already an improvement (it is now on the
                    // wire); letting the author override it needs a new
                    // `#[child(id_field = ..)]` argument, which lives in the
                    // build that owns `parse.rs`.
                    id_field: "name".to_string(),
                    path_template: format!("{child_namespace}/{{id}}"),
                    namespace: child_namespace,
                    child_ty,
                    description,
                }),
            }
        }
    }
}

// ===========================================================================
// Type predicates — all purely syntactic
// ===========================================================================

/// `Some(C)` when `ty` is (a reference to) `Client<C>`.
///
/// A `Client` written without its capability set is an error rather than an
/// empty set: the whole point of the handle is that its type is the
/// declaration, and an inferred one declares nothing.
fn capability_handle_set(ty: &Type) -> syn::Result<Option<Type>> {
    let inner = strip_refs(ty);
    let Type::Path(tp) = inner else {
        return Ok(None);
    };
    let Some(seg) = tp.path.segments.last() else {
        return Ok(None);
    };
    if seg.ident != "Client" {
        return Ok(None);
    }
    match &seg.arguments {
        PathArguments::AngleBracketed(args) => {
            for a in &args.args {
                if let GenericArgument::Type(t) = a {
                    return Ok(Some(t.clone()));
                }
            }
            Err(syn::Error::new(
                seg.span(),
                "`Client<..>` must name its capability set, e.g. \
                 `Client<(Permission, FsRead)>`",
            ))
        }
        _ => Err(syn::Error::new(
            seg.span(),
            "`Client` must name its capability set, e.g. \
             `Client<(Permission, FsRead)>` — an inferred set declares nothing",
        )),
    }
}

/// The `T` of `Option<T>`.
fn option_inner(ty: &Type) -> Option<Type> {
    let Type::Path(tp) = ty else { return None };
    let seg = tp.path.segments.last()?;
    if seg.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
}

/// The `T` of `Result<T, E>`.
fn result_ok(ty: &Type) -> Option<Type> {
    let Type::Path(tp) = ty else { return None };
    let seg = tp.path.segments.last()?;
    if seg.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
}

/// The `T` of a stream type: `impl Stream<Item = T>`, or a `dyn Stream<Item =
/// T>` reached through any number of `Pin` / `Box` wrappers.
fn stream_item(ty: &Type) -> Option<Type> {
    match ty {
        Type::ImplTrait(it) => bounds_stream_item(it.bounds.iter()),
        Type::TraitObject(to) => bounds_stream_item(to.bounds.iter()),
        Type::Paren(p) => stream_item(&p.elem),
        Type::Path(tp) => {
            let seg = tp.path.segments.last()?;
            if seg.ident != "Pin" && seg.ident != "Box" && seg.ident != "Arc" {
                return None;
            }
            let PathArguments::AngleBracketed(args) = &seg.arguments else {
                return None;
            };
            args.args.iter().find_map(|a| match a {
                GenericArgument::Type(t) => stream_item(t),
                _ => None,
            })
        }
        _ => None,
    }
}

fn bounds_stream_item<'a>(bounds: impl Iterator<Item = &'a TypeParamBound>) -> Option<Type> {
    for bound in bounds {
        let TypeParamBound::Trait(tb) = bound else {
            continue;
        };
        let Some(seg) = tb.path.segments.last() else {
            continue;
        };
        if seg.ident != "Stream" {
            continue;
        }
        let PathArguments::AngleBracketed(args) = &seg.arguments else {
            continue;
        };
        for a in &args.args {
            if let GenericArgument::AssocType(assoc) = a {
                if assoc.ident == "Item" {
                    return Some(assoc.ty.clone());
                }
            }
        }
    }
    None
}

/// Peel `&`/`&mut` off a type.
fn strip_refs(ty: &Type) -> &Type {
    match ty {
        Type::Reference(r) => strip_refs(&r.elem),
        Type::Paren(p) => strip_refs(&p.elem),
        other => other,
    }
}

/// Is this the unit type (or `!`)?
pub(crate) fn is_unit(ty: &Type) -> bool {
    match ty {
        Type::Tuple(t) => t.elems.is_empty(),
        Type::Never(_) => true,
        Type::Paren(p) => is_unit(&p.elem),
        _ => false,
    }
}

/// `None` for the unit type — the *only* way a unit reaches the IR, and it
/// reaches it as an absent field rather than a `SchemaRef`.
fn non_unit(ty: Type) -> Option<Type> {
    if is_unit(&ty) {
        None
    } else {
        Some(ty)
    }
}

/// The canonical name recorded in a `SchemaRef`: the author's own type tokens,
/// whitespace-normalized so `Vec < String >` and `Vec<String>` agree.
pub(crate) fn type_name_of(ty: &Type) -> String {
    ty.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}
