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

use super::spec::{
    AuthRequirementSpec, ChildDeliverySpec, ChildEdgeSpec, ChildShapeSpec, ParamIrSpec,
};
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

        // PLX-91: the legacy bidirectional channel handle is the pre-vNext
        // spelling of the capability handle — a framework-supplied duplex
        // channel, never something a caller sends. It is dropped from
        // `params` for exactly the reason `Client<C>` is. Its request/response
        // pair has no `MethodIr` slot (`callbacks` is a capability SET, not a
        // req/resp pair — the same gap PLX-91's requirement 4 records), so the
        // IR records the method without it rather than inventing a schema for
        // a channel type that has none.
        if is_bidir_channel(&pt.ty) {
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
/// # `E` is discarded, and PLX-110 decided to keep it that way
///
/// The `Result` rows drop `E` on the floor, and `MethodIr` has no slot for it —
/// there is no `errors` key in RFC 002 §3.6 item 13's method field list. PLX-110
/// considered adding one and declined: under that ticket's decision the wire
/// never carries `E`, it carries a `plexus_core::runtime::TurnError` whose shape
/// RFC 002 §3.6 item 22 already specifies and which is uniform for every method,
/// so a client generator already knows the failure type. Declaring `E` would
/// declare a *server-side* type no consumer can observe. Giving `details` a
/// declared schema is a genuinely new axis and therefore an RFC amendment; it is
/// filed as a candidate rather than smuggled in as a field.
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

/// PLX-110: the return shape, as **dispatch** must see it.
///
/// [`classify_return`] answers a question about the *IR*: which of `updates` /
/// `terminal` does this signature declare? This enum answers the question the
/// *codegen* has to answer: which body shape can actually drive this return
/// value? They are different questions over the same tokens, and the whole
/// subject of PLX-110 is that they were allowed to disagree — `ir_parse`
/// accepted `Result` from the day it was written (see the doc table above,
/// which has claimed the `Result<impl Stream<Item = T>, E>` row since PLX-79)
/// while `codegen/handlers.rs` emitted one `while let Some(..) = stream.next()`
/// body for every method and therefore refused it.
///
/// Both questions are now answered from the same predicates in this file, so
/// the disagreement cannot silently reappear: a shape `classify_return` maps to
/// `terminal` is exactly the shape [`DispatchShape::UnaryValue`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchShape {
    /// `-> impl Stream<Item = T>` (or a `Pin<Box<dyn Stream<..>>>`). The
    /// pre-PLX-110 arm, and the ONLY arm whose emitted tokens PLX-110 leaves
    /// byte-for-byte alone.
    Stream,
    /// `-> Result<impl Stream<Item = T>, E>`. `signature.rs`'s doc table has
    /// always claimed to classify this; PLX-110's probe showed dispatch
    /// rejected it with the same `Result<..>: Stream is not satisfied` error a
    /// unary method got. It streams, after the `Result` is unwrapped.
    ResultStream,
    /// `-> Result<T, E>` with a non-unit `T`. Zero updates, one terminal
    /// carrying the serialized `T`.
    UnaryValue,
    /// `-> Result<(), E>`. Zero updates, one terminal with **no** value —
    /// RFC 002 §6.4 forbids substituting a unit type for "declares nothing",
    /// so this must not become a terminal carrying `null`.
    UnaryUnit,
    /// Anything else, including a bare `-> T` and a method with no return type
    /// at all. Emitted exactly as before; PLX-110 does not widen this.
    Other,
}

/// Classify a return type for **dispatch**, not for the IR.
pub(crate) fn dispatch_shape(output: &ReturnType) -> DispatchShape {
    let ReturnType::Type(_, ty) = output else {
        return DispatchShape::Other;
    };

    if stream_item(ty).is_some() {
        return DispatchShape::Stream;
    }

    if let Some(ok_ty) = result_ok(ty) {
        if stream_item(&ok_ty).is_some() {
            return DispatchShape::ResultStream;
        }
        if is_unit(&ok_ty) {
            return DispatchShape::UnaryUnit;
        }
        return DispatchShape::UnaryValue;
    }

    DispatchShape::Other
}

/// The `T` of a `-> Result<T, E>` whose `T` is a real value (not a stream, not
/// unit) — the terminal type a unary method declares.
///
/// PLX-110: this is the *only* new consumer of `result_ok` outside
/// [`classify_return`], and it exists so `MethodSchema` can keep advertising a
/// return schema for the new shape (PLX-103 keeps `MethodSchema` alive). It is
/// deliberately `None` for every shape that existed before PLX-110, which is
/// what makes every pre-existing method's schema and hash byte-identical.
pub(crate) fn unary_terminal_type(output: &ReturnType) -> Option<Type> {
    match dispatch_shape(output) {
        DispatchShape::UnaryValue => {
            let ReturnType::Type(_, ty) = output else {
                return None;
            };
            result_ok(ty)
        }
        _ => None,
    }
}

/// Build a [`ChildEdgeSpec`] from a `#[child]`-annotated method.
///
/// Reuses [`ChildMethodInfo::from_fn`] for signature validation and the
/// `list`/`search` arguments, so there is one implementation of the `#[child]`
/// grammar.
///
/// # PLX-160 — the two axes, read independently
///
/// The declaration has **always** carried two orthogonal facts, and this
/// function now reads each exactly once:
///
/// | | `fn ns(&self) -> C` | `fn ns(&self, name: &str) -> Option<C>` |
/// |---|---|---|
/// | bare `#[child]` | single, embedded | single, lazy |
/// | `#[child(list = "…")]` | indexed, embedded | **indexed, lazy** |
///
/// The bottom-right cell is the one this build adds. Before it, the match ran
/// on [`ChildMethodKind`] **first** and consulted `info.list_fn` only inside the
/// `Static` arm, so a `list =` on a dynamic child was parsed and then thrown
/// away with no diagnostic — `list_method`, `id_field` and `path_template` all
/// lost, on `solar.body`, `claudecode.session` and `cone.of`.
pub(crate) fn child_edge(
    namespace: &str,
    f: &ImplItemFn,
    sibling_fns: &std::collections::BTreeSet<String>,
) -> syn::Result<ChildEdgeSpec> {
    let info = ChildMethodInfo::from_fn(f)?;
    let child_namespace = info.fn_name.to_string();
    let description = info.description.clone();

    let ReturnType::Type(_, ret) = &f.sig.output else {
        return Err(syn::Error::new(
            f.sig.span(),
            "a #[child] method must declare a return type",
        ));
    };

    // ---- AXIS 2, DELIVERY: the method's arity, and nothing else. ----------
    //
    // RFC 002 §5.1: "the edge MUST carry the child's namespace and its
    // **advertised** hash". A dynamic child is registered at runtime, but its
    // *shape* is not a runtime fact — `fn ns(&self, name: &str) ->
    // Option<Child>` names the type every registration under `ns` has. So the
    // advertised hash is `Child`'s own node hash, which is what the backend is
    // advertising. §5.2 forbids a *consumer* recomputing it; it does not forbid
    // the declaring backend from declaring it, and emitting an empty string
    // instead is the non-conformance §5.1 names.
    let (delivery, child_ty) = match info.kind {
        ChildMethodKind::Static => (ChildDeliverySpec::Embedded, (**ret).clone()),
        ChildMethodKind::Dynamic => (
            ChildDeliverySpec::Lazy,
            option_inner(ret).unwrap_or_else(|| (**ret).clone()),
        ),
    };

    // PLX-160 c3 — NOTE, and it corrects the ticket's premise. `list` and
    // `search` naming a method the impl block does not define is ALREADY a
    // compile error, raised by the `ChildRouter` emitter, and it fires for a
    // dynamic child too. This build did not need to add that check — but it is
    // what made it load-bearing on the WIRE: before this build the name was
    // validated and then thrown away, so the diagnostic guarded a routing call
    // and nothing else. It now guards RFC 002 §5.1's mandatory enumeration
    // method, which a consumer is told it may call. Pinned by
    // `tests/compile/plx160_list_names_nothing.rs`.

    // ---- AXIS 1, SHAPE: the `list` / `search` arguments, and nothing else. -
    let shape = match (&info.list_fn, &info.search_fn) {
        (Some(list_fn), search_fn) => ChildShapeSpec::Indexed {
            list_method: format!("{namespace}.{list_fn}"),
            search_method: search_fn.as_ref().map(|s| format!("{namespace}.{s}")),
            // PLX-73 `q-ir-completeness` item 17: the old client *guessed* this
            // field with a `"name"` fallback. Declaring the guess is already an
            // improvement (it is now on the wire); letting the author override
            // it needs a new `#[child(id_field = ..)]` argument, which lives in
            // the build that owns `parse.rs`.
            id_field: "name".to_string(),
            path_template: format!("{child_namespace}/{{id}}"),
        },
        (None, None) => ChildShapeSpec::Single,
        // `search` WITHOUT `list` is not an error and is not dropped: CHILD-4
        // made the two opt-ins independent, and this declaration is honoured in
        // full on the ROUTING surface (`ChildRouter::search_children` works,
        // `list_children` stays `None`, capabilities are `SEARCH`).
        //
        // What it cannot do is ride an IR edge: RFC 002 §5.1 makes the
        // enumeration method MANDATORY for an indexed edge and the search method
        // the only optional one of its five facts, so a searchable-but-not-
        // enumerable family has no §5.1 form. The edge is therefore Single and
        // the search method is absent from the DOCUMENT while present in the
        // BACKEND — a §5.1 gap, not a macro bug, and one this build reports
        // rather than papers over. `search_only_is_honoured_by_the_router_and_
        // unrepresentable_in_the_document` pins it so it stays loud.
        (None, _) => ChildShapeSpec::Single,
    };

    Ok(ChildEdgeSpec {
        namespace: child_namespace,
        shape,
        delivery,
        child_ty,
        description,
    })
}

// ===========================================================================
// Type predicates — all purely syntactic
// ===========================================================================

/// `Some(C)` when `ty` is (a reference to) `Client<C>`.
///
/// A `Client` written without its capability set is an error rather than an
/// empty set: the whole point of the handle is that its type is the
/// declaration, and an inferred one declares nothing.
pub(crate) fn capability_handle_set(ty: &Type) -> syn::Result<Option<Type>> {
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

/// Is this (a reference to, or an `Arc`/`Box`/`Rc` around) a `BidirChannel<..>`?
fn is_bidir_channel(ty: &Type) -> bool {
    let inner = strip_refs(ty);
    let Type::Path(tp) = inner else { return false };
    let Some(seg) = tp.path.segments.last() else {
        return false;
    };
    // PLX-104: the legacy path (parse.rs:1498) accepts both spellings; the vNext
    // parser matched only the bare ident, so a `StandardBidirChannel` ctx param
    // was treated as an ordinary param and its method failed to compile.
    if seg.ident == "BidirChannel" || seg.ident == "StandardBidirChannel" {
        return true;
    }
    if matches!(seg.ident.to_string().as_str(), "Arc" | "Rc" | "Box") {
        if let PathArguments::AngleBracketed(args) = &seg.arguments {
            return args.args.iter().any(|a| match a {
                GenericArgument::Type(t) => is_bidir_channel(t),
                _ => false,
            });
        }
    }
    false
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
///
/// PLX-110 made this `pub(crate)`: `parse.rs` needs it to peel the `Result` off
/// a `Result<impl Stream<Item = T>, E>` before looking for the stream item, and
/// there must be exactly one implementation of "what is the Ok type" in the
/// crate. `E` is discarded here and nowhere recorded — see the report's
/// error-type decision, and `classify_return`'s doc table.
pub(crate) fn result_ok(ty: &Type) -> Option<Type> {
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
