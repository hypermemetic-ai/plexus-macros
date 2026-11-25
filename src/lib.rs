//! `#[activation]` on a module: the module's types and its one `impl` block
//! are read into an IR (`plexus-ir`), and from the IR `plexus-gen` writes its
//! WIT and its TypeScript.
//!
//! Two forms:
//!
//! - `#[activation]`: built to `wasm32`, the module also becomes a
//!   WebAssembly component that exports the WIT. The activation is made
//!   once, with `Default`, and its methods take `&self`. An `async fn` is an
//!   `async func`; a method that returns `impl Stream<Item = T>` gives a
//!   `stream<T>`. Data types are replaced by the ones wit-bindgen makes from
//!   the WIT. Built natively, the data types stay as written, with serde's
//!   derives by the value layout, and the module gains `serve(activation,
//!   server, at)` and `impl plexus::Serve`: the same methods on a
//!   `plexus::Server`, for `plexus::main`.
//! - `#[activation(describe)]`: the module is left as ordinary Rust and gains
//!   `IR`, `HASH`, `WIT` and `CLIENT_TS`. Everything the IR can say is read; nothing is
//!   wired to run. Native serving supports callers, contexts, mounted children, and
//!   credit-paced input streams. Progress is currently descriptive.

use heck::ToKebabCase;
use plexus_ir::{Caller, Child, Deprecated, Field, Held, Ir, Method, Returns, Role, Shape, Ty, TypeDef};
use serde_json::Value;
use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Error, Expr, Fields, FnArg, GenericArgument, ImplItem, Item, ItemMod, Lit, Meta, MetaNameValue, PathArguments, ReturnType, Token, Type, TypeParamBound};

mod watch_sources;
mod method_args;
use method_args::MethodArgs;

type Res<T> = Result<T, Error>;

#[proc_macro_attribute]
pub fn activation(args: TokenStream, input: TokenStream) -> TokenStream {
    let module = syn::parse_macro_input!(input as ItemMod);
    expand(args.into(), module).map(|(_, tokens)| tokens).unwrap_or_else(|e| e.to_compile_error()).into()
}

#[proc_macro_derive(Context, attributes(from))]
pub fn context(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive_context(input).unwrap_or_else(|e| e.to_compile_error()).into()
}

fn from_caller(field: &syn::Field) -> Res<bool> {
    let mut sources = field.attrs.iter().filter(|attr| attr.path().is_ident("from"));
    let Some(source) = sources.next() else { return Ok(false) };
    if let Some(duplicate) = sources.next() {
        return err(duplicate, "a context field has only one source");
    }
    if source.parse_args::<syn::Ident>()? != "caller" {
        return err(source, "the context field source is #[from(caller)]");
    }
    Ok(true)
}

fn derive_context(input: syn::DeriveInput) -> Res<proc_macro2::TokenStream> {
    let syn::Data::Struct(data) = &input.data else { return err(&input, "a context is a struct with named fields") };
    let Fields::Named(fields) = &data.fields else { return err(&input, "a context is a struct with named fields") };
    let name = &input.ident;
    let activation = format_ident!("__PlexusActivation");
    let mut generics = input.generics.clone();
    generics.params.push(syn::parse_quote!(#activation));
    let (_, type_generics, _) = input.generics.split_for_impl();
    let (mut keys, mut initializers) = (Vec::new(), Vec::new());
    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named");
        let ty = &field.ty;
        let value = if from_caller(field)? {
            generics.make_where_clause().predicates.push(syn::parse_quote!(#ty: ::plexus::FromCaller<#activation>));
            quote!(<#ty as ::plexus::FromCaller<#activation>>::from_caller(activation, ::plexus::require_caller(caller)?)?)
        } else {
            let key = kebab(ident);
            if keys.contains(&key) {
                return Err(Error::new(ident.span(), format!("duplicate context key `{key}`")));
            }
            keys.push(key.clone());
            generics.make_where_clause().predicates.push(syn::parse_quote!(#ty: ::std::str::FromStr));
            quote!(keys.parse::<#ty>(#key)?)
        };
        initializers.push(quote!(#ident: #value));
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::plexus::Context<#activation> for #name #type_generics #where_clause {
            const KEYS: &'static [&'static str] = &[#(#keys),*];
            fn from_context(
                activation: &#activation,
                caller: ::std::option::Option<&::plexus::Caller>,
                keys: &::plexus::ContextKeys,
            ) -> ::std::result::Result<Self, ::plexus::Failure> {
                Ok(Self { #(#initializers),* })
            }
        }
    })
}

fn err<T>(at: impl Spanned, msg: &str) -> Res<T> {
    Err(Error::new(at.span(), msg))
}

fn kebab(ident: &syn::Ident) -> String {
    ident.to_string().to_kebab_case()
}

/// An item's `///` lines, joined.
fn doc(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs {
        if let Meta::NameValue(nv) = &a.meta {
            if nv.path.is_ident("doc") {
                if let Expr::Lit(l) = &nv.value {
                    if let Lit::Str(s) = &l.lit {
                        lines.push(s.value().trim().to_string());
                    }
                }
            }
        }
    }
    lines.join("\n")
}

/// The attributes this macro reads and Rust does not know.
const OURS: [&str; 6] = ["default", "target", "child", "limit", "open", "watch"];

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| a.path().is_ident(name))
}

/// A literal as a JSON value.
fn literal(e: &Expr) -> Res<Value> {
    let number = |digits: String| serde_json::from_str::<serde_json::Number>(&digits).map(Value::Number).or_else(|_| err(e, "a number JSON can hold"));
    match e {
        Expr::Lit(l) => match &l.lit {
            Lit::Str(s) => Ok(Value::String(s.value())),
            Lit::Bool(b) => Ok(Value::Bool(b.value)),
            Lit::Int(i) => number(i.base10_digits().to_string()),
            Lit::Float(f) => number(f.base10_digits().to_string()),
            other => err(other, "a string, a number or a bool"),
        },
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => match literal(&u.expr)? {
            Value::Number(n) => number(format!("-{n}")),
            _ => err(e, "a number after `-`"),
        },
        other => err(other, "a literal"),
    }
}

/// `name = value, ..` inside an attribute's parentheses.
fn pairs(a: &Attribute) -> Res<Vec<(String, Expr)>> {
    let list = a.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated)?;
    Ok(list.into_iter().map(|nv| (nv.path.get_ident().map(|i| i.to_string()).unwrap_or_default(), nv.value)).collect())
}

fn string(e: &Expr) -> Res<String> {
    if let Expr::Lit(l) = e {
        if let Lit::Str(s) = &l.lit {
            return Ok(s.value());
        }
    }
    err(e, "a string")
}

/// `#[default(..)]` and `#[deprecated(since = "..", note = "..")]` of a field or a parameter.
fn extras(attrs: &[Attribute], field: &mut Field) -> Res<()> {
    if let Some(a) = attr(attrs, "default") {
        field.default = Some(literal(&a.parse_args::<Expr>()?)?);
    }
    if let Some(a) = attr(attrs, "deprecated") {
        let mut d = Deprecated { since: String::new(), note: String::new() };
        if matches!(a.meta, Meta::List(_)) {
            for (k, v) in pairs(a)? {
                match k.as_str() {
                    "since" => d.since = string(&v)?,
                    "note" => d.note = string(&v)?,
                    _ => {}
                }
            }
        }
        field.deprecated = Some(d);
    }
    Ok(())
}

fn type_args(args: &PathArguments) -> Vec<&Type> {
    match args {
        PathArguments::AngleBracketed(a) => a.args.iter().filter_map(|g| if let GenericArgument::Type(t) = g { Some(t) } else { None }).collect(),
        _ => Vec::new(),
    }
}

/// The last segment of a type's path, through any `&`: `Caller` of `&plexus::Caller`.
fn last_segment(t: &Type) -> Option<&syn::PathSegment> {
    match t {
        Type::Reference(r) => last_segment(&r.elem),
        Type::Path(p) => p.path.segments.last(),
        _ => None,
    }
}

/// A Rust type as the IR's.
fn ty(t: &Type) -> Res<Ty> {
    match t {
        Type::Reference(r) => ty(&r.elem),
        Type::Tuple(tu) if tu.elems.is_empty() => err(t, "`()` is no type of an interface; leave the return off"),
        Type::Tuple(tu) => Ok(Ty::Tuple(tu.elems.iter().map(ty).collect::<Res<_>>()?)),
        Type::Path(p) => {
            let last = p.path.segments.last().expect("a path has a segment");
            let args = type_args(&last.arguments);
            let nth = |n: usize| args.get(n).copied().ok_or_else(|| Error::new(t.span(), "a type argument is missing"));
            Ok(match last.ident.to_string().as_str() {
                "String" | "str" => Ty::Prim("string"),
                "bool" => Ty::Prim("bool"),
                "char" => Ty::Prim("char"),
                "u8" => Ty::Prim("u8"),
                "u16" => Ty::Prim("u16"),
                "u32" => Ty::Prim("u32"),
                "u64" => Ty::Prim("u64"),
                "i8" => Ty::Prim("s8"),
                "i16" => Ty::Prim("s16"),
                "i32" => Ty::Prim("s32"),
                "i64" => Ty::Prim("s64"),
                "f32" => Ty::Prim("f32"),
                "f64" => Ty::Prim("f64"),
                "Bytes" => Ty::Prim("bytes"),
                "Json" => Ty::Prim("json"),
                "Vec" => match ty(nth(0)?)? {
                    Ty::Prim("u8") => Ty::Prim("bytes"),
                    item => Ty::List(Box::new(item)),
                },
                "Option" => Ty::Option(Box::new(ty(nth(0)?)?)),
                "BTreeMap" | "HashMap" => {
                    if ty(nth(0)?)? != Ty::Prim("string") {
                        return err(t, "a map is keyed by String");
                    }
                    Ty::Map(Box::new(ty(nth(1)?)?))
                }
                "usize" | "isize" => return err(t, "its size differs by machine: say u32 or u64"),
                _ => Ty::Named(kebab(&last.ident)),
            })
        }
        _ => err(t, "this type has no form in an interface yet"),
    }
}

/// The `T` of `impl Stream<Item = T>` (or of `impl Iterator<Item = T>`,
/// for a stream that never waits), when a method returns one.
fn stream_item(ret: &ReturnType) -> Option<Type> {
    stream_of(ret).map(|(item, _)| item)
}

/// The same, and whether it is an iterator.
fn stream_of(ret: &ReturnType) -> Option<(Type, bool)> {
    let ReturnType::Type(_, t) = ret else { return None };
    let ok = result_of(t).ok()?.map_or(&**t, |(ok, _)| ok);
    let Type::ImplTrait(it) = ok else { return None };
    it.bounds.iter().find_map(|b| {
        let TypeParamBound::Trait(t) = b else { return None };
        let last = t.path.segments.last()?;
        if last.ident != "Iterator" && last.ident != "Stream" {
            return None;
        }
        let iterator = last.ident == "Iterator";
        let PathArguments::AngleBracketed(a) = &last.arguments else { return None };
        a.args.iter().find_map(|g| if let GenericArgument::AssocType(at) = g { (at.ident == "Item").then(|| (at.ty.clone(), iterator)) } else { None })
    })
}

/// `Result<T, E>` as its two types.
fn result_of(t: &Type) -> Res<Option<(&Type, &Type)>> {
    let Type::Path(p) = t else { return Ok(None) };
    let last = p.path.segments.last().expect("a path has a segment");
    if last.ident != "Result" {
        return Ok(None);
    }
    let args = type_args(&last.arguments);
    if args.len() != 2 {
        return err(t, "a method returns Result<T, E>: both, so the error is part of the interface");
    }
    Ok(Some((args[0], args[1])))
}

/// A native encoded answer advertises the payload type in the interface.
fn encoded_inner(t: &Type) -> Option<&Type> {
    let Type::Path(path) = t else { return None };
    let segments = &path.path.segments;
    let last = segments.last()?;
    if last.ident != "Encoded" || !(segments.len() == 1 || (segments.len() == 2 && segments[0].ident == "plexus")) {
        return None;
    }
    let args = type_args(&last.arguments);
    match args.as_slice() { [inner] => Some(*inner), _ => None }
}

fn encoded_output(ret: &ReturnType) -> bool {
    if let Some(item) = stream_item(ret) { return encoded_inner(&item).is_some(); }
    let ReturnType::Type(_, t) = ret else { return false };
    encoded_inner(result_of(t).ok().flatten().map_or(&**t, |(ok, _)| ok)).is_some()
}

/// What a method gives back, and the error it can end with.
fn returns(ret: &ReturnType) -> Res<(Returns, Option<Ty>)> {
    let ReturnType::Type(_, t) = ret else { return Ok((Returns::Nothing, None)) };
    if let Some(item) = stream_item(ret) {
        let fails = result_of(t)?.map(|(_, error)| ty(error)).transpose()?;
        return Ok((Returns::Stream(ty(encoded_inner(&item).unwrap_or(&item))?), fails));
    }
    if let Some((ok, e)) = result_of(t)? {
        let ok = match ok {
            Type::Tuple(tu) if tu.elems.is_empty() => Returns::Nothing,
            other => Returns::One(ty(encoded_inner(other).unwrap_or(other))?),
        };
        return Ok((ok, Some(ty(e)?)));
    }
    Ok((Returns::One(ty(encoded_inner(t).unwrap_or(t))?), None))
}

/// Where each name a role holds is written: the role, the name as the IR
/// keeps it, and its span.
type RoleSpans = Vec<(String, String, proc_macro2::Span)>;

/// `roles(viewer = ["list"], editor = [viewer, "open"])` of the attribute.
fn roles(args: proc_macro2::TokenStream, activation: &str) -> Res<(bool, Vec<Role>, RoleSpans, Option<syn::Ident>)> {
    let metas = syn::parse::Parser::parse2(Punctuated::<Meta, Token![,]>::parse_terminated, args)?;
    let (mut describe, mut out, mut spans, mut context) = (false, Vec::new(), Vec::new(), None);
    for m in metas {
        match &m {
            Meta::Path(p) if p.is_ident("describe") => describe = true,
            Meta::List(l) if l.path.is_ident("roles") => {
                for nv in l.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated)? {
                    let Expr::Array(items) = &nv.value else { return err(&nv.value, "a role is a list: [\"method\", other_role]") };
                    let mut role = Role { name: nv.path.get_ident().map(kebab).unwrap_or_default(), includes: Vec::new(), grants: Vec::new() };
                    for e in &items.elems {
                        let name = match e {
                            Expr::Path(p) => p.path.get_ident().map(kebab).unwrap_or_default(),
                            other => format!("{activation}.{}", string(other)?.to_kebab_case()),
                        };
                        spans.push((role.name.clone(), name.clone(), e.span()));
                        if matches!(e, Expr::Path(_)) { role.includes.push(name) } else { role.grants.push(name) }
                    }
                    out.push(role);
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("context") => {
                let Expr::Path(path) = &nv.value else { return err(&nv.value, "context names a struct in this activation") };
                let Some(name) = path.path.get_ident() else { return err(path, "context names a struct in this activation") };
                if context.replace(name.clone()).is_some() {
                    return err(nv, "an activation has only one context");
                }
            }
            other => return err(other, "`describe`, `roles(..)` or `context = Type`"),
        }
    }
    Ok((describe, out, spans, context))
}

/// The module with this macro's own attributes taken off, so it is plain Rust.
fn plain(items: &[Item]) -> Vec<Item> {
    fn strip(attrs: &mut Vec<Attribute>, param: bool) {
        attrs.retain(|a| !OURS.iter().any(|o| a.path().is_ident(o)) && !(param && a.path().is_ident("deprecated")));
    }
    let mut items = items.to_vec();
    for item in &mut items {
        match item {
            Item::Struct(s) => s.fields.iter_mut().for_each(|f| strip(&mut f.attrs, false)),
            Item::Enum(e) => {
                for v in e.variants.iter_mut() {
                    // Only `#[wire]`: a variant's `#[default]` is Rust's own.
                    v.attrs.retain(|a| !a.path().is_ident("wire"));
                    v.fields.iter_mut().for_each(|f| strip(&mut f.attrs, false));
                }
            }
            Item::Type(t) => strip(&mut t.attrs, false),
            Item::Impl(i) => {
                for it in &mut i.items {
                    let ImplItem::Fn(f) = it else { continue };
                    strip(&mut f.attrs, false);
                    for arg in f.sig.inputs.iter_mut() {
                        if let FnArg::Typed(t) = arg {
                            strip(&mut t.attrs, true);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    items
}

fn fields_of(named: &syn::FieldsNamed) -> Res<Vec<Field>> {
    named.named.iter().map(|f| {
        let mut field = Field { name: kebab(f.ident.as_ref().expect("named")), doc: doc(&f.attrs), ty: Some(ty(&f.ty)?), ..Field::default() };
        extras(&f.attrs, &mut field)?;
        Ok(field)
    }).collect()
}

/// The context and its optional preflight method, neither of which is on the wire.
struct ActivationContext<'a> {
    name: &'a syn::Ident,
    open: Option<&'a syn::ImplItemFn>,
    ir: plexus_ir::Context,
    caller_types: Vec<syn::Ident>,
    key_types: Vec<proc_macro2::TokenStream>,
    requires_caller: bool,
}

fn token_mentions(tokens: proc_macro2::TokenStream, name: &syn::Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(i) => i == *name,
        proc_macro2::TokenTree::Group(g) => token_mentions(g.stream(), name),
        _ => false,
    })
}

fn data_name(item: &Item) -> Option<&syn::Ident> {
    match item {
        Item::Struct(s) => Some(&s.ident),
        Item::Enum(e) => Some(&e.ident),
        Item::Type(t) => Some(&t.ident),
        _ => None,
    }
}

fn private_context_types<'a>(
    context: &ActivationContext<'_>,
    activation: &syn::Ident,
    items: &'a [Item],
    imp: &syn::ItemImpl,
) -> Vec<&'a syn::Ident> {
    // Ordinary data definitions and method signatures still expose types
    // that are also used in the context.
    let mut wire: Vec<_> = imp.items.iter().filter_map(|item| match item {
        ImplItem::Fn(f) if matches!(f.vis, syn::Visibility::Public(_))
            || attr(&f.attrs, "open").is_some() => Some(f.sig.to_token_stream()),
        _ => None,
    }).collect();
    wire.extend(context.key_types.iter().cloned());
    let mut candidates = Vec::new();
    for item in items {
        let Some(name) = data_name(item) else { continue };
        if name == context.name || name == activation {
            continue;
        }
        let tokens = item.to_token_stream();
        if context.caller_types.contains(name) {
            candidates.push((name, tokens));
        } else {
            wire.push(tokens);
        }
    }
    // Include dependencies of any otherwise context-only type used on the wire.
    while let Some(i) = candidates.iter().position(|(name, _)|
        wire.iter().any(|tokens| token_mentions(tokens.clone(), name))) {
        wire.push(candidates.swap_remove(i).1);
    }
    candidates.into_iter().map(|(name, _)| name).collect()
}

fn is_named(t: &Type, name: &syn::Ident) -> bool {
    last_segment(t).is_some_and(|s| s.ident == *name)
}

fn is_shared_ref(t: &Type, name: &syn::Ident) -> bool {
    matches!(t, Type::Reference(r) if r.mutability.is_none() && matches!(&*r.elem, Type::Path(_)) && is_named(&r.elem, name))
}

fn activation_context<'a>(name: Option<&'a syn::Ident>, items: &'a [Item], imp: &'a syn::ItemImpl) -> Res<Option<ActivationContext<'a>>> {
    let opens = imp.items.iter().filter_map(|item| match item {
        ImplItem::Fn(f) if attr(&f.attrs, "open").is_some() => Some(f),
        _ => None,
    }).collect::<Vec<_>>();
    let Some(name) = name else {
        return match opens.first() {
            Some(f) => err(&f.sig, "#[open] requires #[activation(context = Type)]"),
            None => Ok(None),
        };
    };
    let found = items.iter().find_map(|item| match item {
        Item::Struct(s) if s.ident == *name => Some(s),
        _ => None,
    });
    let Some(found) = found else { return err(name, "declare the context struct inside this activation") };
    let Fields::Named(fields) = &found.fields else { return err(found, "a context is a struct with named fields") };
    let mut context = ActivationContext {
        name,
        open: opens.first().copied(),
        ir: plexus_ir::Context { keys: Vec::new(), fails: None },
        caller_types: Vec::new(),
        key_types: Vec::new(),
        requires_caller: false,
    };
    for field in &fields.named {
        if from_caller(field)? {
            context.requires_caller = true;
            context.caller_types.extend(last_segment(&field.ty).map(|s| s.ident.clone()));
        } else {
            let ident = field.ident.as_ref().expect("named");
            let name = kebab(ident);
            if context.ir.keys.iter().any(|key| key.name == name) {
                return Err(Error::new(ident.span(), format!("duplicate context key `{name}`")));
            }
            context.key_types.push(field.ty.to_token_stream());
            context.ir.keys.push(Field { name, doc: doc(&field.attrs), ty: Some(ty(&field.ty)?), ..Field::default() });
        }
    }
    if opens.len() > 1 {
        return err(&opens[1].sig, "an activation has only one #[open]");
    }
    if let Some(f) = context.open {
        let (recv, ctx) = (f.sig.inputs.first(), f.sig.inputs.iter().nth(1));
        let valid_recv = matches!(recv, Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() && r.colon_token.is_none());
        let valid_ctx = matches!(ctx, Some(FnArg::Typed(t)) if is_shared_ref(&t.ty, name));
        if f.sig.asyncness.is_some() || !valid_recv || !valid_ctx || f.sig.inputs.len() != 2 {
            return err(&f.sig, "#[open] takes &self and a shared reference to the activation context");
        }
        let ReturnType::Type(_, output) = &f.sig.output else { return err(&f.sig, "#[open] returns Result<(), E>") };
        let Some((ok, failure)) = result_of(output)? else { return err(output, "#[open] returns Result<(), E>") };
        if !matches!(ok, Type::Tuple(t) if t.elems.is_empty()) {
            return err(ok, "#[open] returns Result<(), E>");
        }
        context.ir.fails = Some(ty(failure)?);
    }
    Ok(Some(context))
}

/// The module's IR, and the module as a component (or, described, as itself).
fn expand(args: proc_macro2::TokenStream, module: ItemMod) -> Res<(Ir, proc_macro2::TokenStream)> {
    let name = kebab(&module.ident);
    let (describe, roles, role_spans, context_name) = roles(args, &name)?;
    let Some((_, items)) = &module.content else { return err(&module, "#[activation] goes on `mod name { .. }` with its body") };
    let Some(imp) = items.iter().find_map(|i| if let Item::Impl(i) = i { i.trait_.is_none().then_some(i) } else { None }) else {
        return err(&module, "the module needs one `impl` block: the activation's methods");
    };
    let Type::Path(self_ty) = &*imp.self_ty else { return err(&imp.self_ty, "the activation is a named type") };
    let activation = self_ty.path.segments.last().expect("a path has a segment").ident.clone();
    let context = activation_context(context_name.as_ref(), items, imp)?;
    let mut ir = Ir { activation: name.clone(), doc: doc(&module.attrs), types: Vec::new(), methods: Vec::new(), children: Vec::new(), roles, context: context.as_ref().map(|c| c.ir.clone()) };

    // Data: every other struct, enum and alias goes into the IR.
    let (mut kept, mut names, mut data) = (Vec::new(), Vec::new(), Vec::new());
    let private_context = context.as_ref().map(|c| private_context_types(c, &activation, items, imp)).unwrap_or_default();
    for item in items {
        if data_name(item).is_some_and(|n| context.as_ref().is_some_and(|c| n == c.name) || private_context.contains(&n)) {
            kept.push(item.clone());
            continue;
        }
        match item {
            // A struct with no fields is no data: an activation of its own, held as a child.
            Item::Struct(s) if s.ident != activation && !matches!(s.fields, Fields::Unit) => {
                let Fields::Named(named) = &s.fields else { return err(s, "a record has named fields") };
                ir.types.push(TypeDef { name: kebab(&s.ident), doc: doc(&s.attrs), shape: Shape::Record(fields_of(named)?) });
                names.push(s.ident.clone());
                data.push(item.clone());
            }
            Item::Enum(e) => {
                let mut cases = Vec::new();
                for v in &e.variants {
                    let mut case = Field { name: kebab(&v.ident), doc: doc(&v.attrs), ..Field::default() };
                    if let Some(a) = attr(&v.attrs, "wire") {
                        case.wire = Some(string(&a.parse_args::<Expr>()?)?);
                    }
                    match &v.fields {
                        Fields::Unit => {}
                        Fields::Unnamed(u) if u.unnamed.len() == 1 => case.ty = Some(ty(&u.unnamed[0].ty)?),
                        Fields::Named(named) => case.record = Some(fields_of(named)?),
                        _ => return err(v, "a case carries nothing, one value, or named fields"),
                    }
                    cases.push(case);
                }
                let shape = if cases.iter().all(|c| c.ty.is_none() && c.record.is_none()) { Shape::Enum(cases) } else { Shape::Variant(cases) };
                ir.types.push(TypeDef { name: kebab(&e.ident), doc: doc(&e.attrs), shape });
                names.push(e.ident.clone());
                data.push(item.clone());
            }
            Item::Type(t) => {
                let mut limits = Vec::new();
                if let Some(a) = attr(&t.attrs, "limit") {
                    for (k, v) in pairs(a)? {
                        limits.push((k.to_kebab_case(), literal(&v)?));
                    }
                }
                ir.types.push(TypeDef { name: kebab(&t.ident), doc: doc(&t.attrs), shape: Shape::Alias(ty(&t.ty)?, limits) });
                names.push(t.ident.clone());
                data.push(item.clone());
            }
            other => kept.push(other.clone()),
        }
    }

    // Methods and children: every `pub fn` taking `&self` or `&mut self`.
    let iface = format_ident!("{}", module.ident);
    let exports = quote!(self::exports::plexus::#iface::#iface);
    let (mut glue, mut served, mut mounted) = (Vec::new(), Vec::new(), Vec::new());
    let mut subscriptions = Vec::new();
    let mut has_encoded = false;
    for item in &imp.items {
        let ImplItem::Fn(f) = item else { continue };
        if attr(&f.attrs, "open").is_some() || !matches!(f.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let Some(FnArg::Receiver(recv)) = f.sig.inputs.first() else { continue };
        if recv.mutability.is_some() && !describe {
            return err(recv, "an activation's methods take `&self`: two calls can be running at once. Keep what changes in a Cell or a RefCell");
        }
        let method = &f.sig.ident;
        let mut m = Method { watch: None, name: kebab(method), doc: doc(&f.attrs), params: Vec::new(), returns: Returns::Nothing, fails: None, input: None, progress: None, caller: None, grant: None, target: None, waits: f.sig.asyncness.is_some() || stream_item(&f.sig.output).is_some() };
        let args = MethodArgs::read(&f.sig, &mut m, context.as_ref().map(|c| c.name), describe)?;
        if let Some(a) = attr(&f.attrs, "child") {
            if m.input.is_some() {
                return err(&f.sig, "a child factory cannot take an input stream");
            }
            if attr(&f.attrs, "watch").is_some() { return err(f, "a child cannot be a watchable read"); }
            // A child: the activation this function hands back.
            let ReturnType::Type(_, ret) = &f.sig.output else { return err(&f.sig, "a child function returns the child") };
            let (held, fails) = match result_of(ret)? {
                Some((ok, e)) => (ok, Some(ty(e)?)),
                None => (&**ret, None),
            };
            // `&Commits` is an activation written here. `Box<dyn Forge>` is anything
            // that fits an interface. `Box<dyn Activation>` is anything at all.
            let Some(seg) = last_segment(held) else { return err(held, "a child is a named activation") };
            let boxed = (seg.ident == "Box").then(|| type_args(&seg.arguments).first().copied()).flatten().and_then(|t| match t {
                Type::TraitObject(o) => o.bounds.iter().find_map(|b| if let TypeParamBound::Trait(t) = b { t.path.segments.last().map(|s| s.ident.clone()) } else { None }),
                _ => None,
            });
            let (activation, how) = match &boxed {
                Some(i) if i == "Activation" => (String::new(), Held::Any),
                Some(i) => (kebab(i), Held::Interface),
                None => (kebab(&seg.ident), Held::Fixed),
            };
            let mut child = Child { name: m.name, doc: m.doc, activation, held: how, key: m.params.into_iter().next(), list: None, search: None, fails };
            if matches!(a.meta, Meta::List(_)) {
                for (k, v) in pairs(a)? {
                    match k.as_str() {
                        "list" => child.list = Some(string(&v)?.to_kebab_case()),
                        "search" => child.search = Some(string(&v)?.to_kebab_case()),
                        _ => return err(a, "`list = \"..\"` or `search = \"..\"`"),
                    }
                }
            }
            if !describe {
                if args.takes_context || m.caller.is_some() {
                    return err(&f.sig, "a mounted child factory cannot take a context or caller");
                }
                mounted.push(mounted_child(&child, f, held)?);
            }
            ir.children.push(child);
            continue;
        }
        (m.returns, m.fails) = returns(&f.sig.output)?;
        let mut sources = Vec::new();
        if let Some(a) = attr(&f.attrs, "watch") {
            if !matches!(m.returns, Returns::One(_)) || m.input.is_some() || m.progress.is_some() {
                return err(a, "watch requires a unary read with one result and no input stream or progress");
            }
            let fields = pairs(a)?;
            let [(on, Expr::Array(deps))] = fields.as_slice() else { return err(a, "expected #[watch(on = [\"dependency\"])]") };
            if on != "on" || deps.elems.is_empty() { return err(a, "watch requires a nonempty list of dependencies"); }
            let mut names = Vec::new();
            for dep in &deps.elems {
                let name = if matches!(dep, Expr::Lit(_)) {
                    string(dep)?
                } else {
                    let (name, source) = watch_sources::call(dep)?;
                    sources.push(source);
                    name
                };
                if name.trim().is_empty() || names.contains(&name) { return err(dep, "empty or duplicate watch dependency"); }
                names.push(name);
            }
            if !sources.is_empty() {
                if sources.len() != names.len() { return err(a, "use either source method calls or named dependencies in one watch"); }
                subscriptions.push(watch_sources::helper(f, &sources));
            }
            m.watch = Some(plexus_ir::Watch { on: names, stream: format!("{}-watch", m.name) });
        }
        has_encoded |= encoded_output(&f.sig.output);
        if !ir.roles.is_empty() || m.target.is_some() {
            m.grant = Some(format!("{name}.{}", m.name));
        }
        if !describe {
            served.push(served_method(&ir, &m, f, &args, context.as_ref(), &activation, !sources.is_empty()));
        }
        if context.as_ref().is_some_and(|c| c.requires_caller) {
            m.caller = Some(Caller::Required);
        }
        if !describe && m.caller.is_none() && context.is_none() && m.input.is_none() {
            let (idents, tys) = (&args.wire_names, &args.wire_types);
            let out = &f.sig.output;
            if let Some((item, iterator)) = stream_of(&f.sig.output) {
                // A stream out: a task of its own walks the method's stream
                // and writes each item into WIT's. The write waits for the
                // reader, and a reader that has gone ends the walk.
                let walk = if iterator { quote!(::futures::stream::iter(made)) } else { quote!(made) };
                let wait = f.sig.asyncness.is_some().then(|| quote!(.await));
                let call = quote!(activation.#method(#(#idents),*) #wait);
                let (out, channel, make, finish) = if let ReturnType::Type(_, ty) = &f.sig.output
                    && let Some((_, error)) = result_of(ty)? {
                    (
                        quote!(Result<::wit_bindgen::StreamReader<#item>, #error>),
                        quote!(let (opened, ready) = ::futures::channel::oneshot::channel();),
                        quote!(match #call {
                            Ok(items) => { if opened.send(Ok(())).is_err() { return; } items }
                            Err(e) => { let _ = opened.send(Err(e)); return; }
                        }),
                        quote!(ready.await.expect("stream opening task ended")?; Ok(rx)),
                    )
                } else {
                    (quote!(::wit_bindgen::StreamReader<#item>), quote!(), call, quote!(rx))
                };
                glue.push(quote! {
                    async fn #method(#(#idents: #tys),*) -> #out {
                        let (mut tx, rx) = self::wit_stream::new();
                        let activation = __ACTIVATION.with(::std::clone::Clone::clone);
                        #channel
                        ::wit_bindgen::spawn_local(async move {
                            let made = #make;
                            let mut items = ::std::pin::pin!(#walk);
                            while let ::std::option::Option::Some(item) = ::futures::StreamExt::next(&mut items).await {
                                if !tx.write_all(::std::vec![item]).await.is_empty() { break; }
                            }
                        });
                        #finish
                    }
                });
            } else if f.sig.asyncness.is_some() {
                glue.push(quote! {
                    async fn #method(#(#idents: #tys),*) #out {
                        let activation = __ACTIVATION.with(::std::clone::Clone::clone);
                        activation.#method(#(#idents),*).await
                    }
                });
            } else {
                glue.push(quote!(fn #method(#(#idents: #tys),*) #out { __ACTIVATION.with(|s| s.#method(#(#idents),*)) }));
            }
        }
        // WIT keeps one namespace for an interface's types and its functions.
        if let Some(at) = ir.types.iter().position(|t| t.name == m.name) {
            let mut e = Error::new(method.span(), format!("method `{method}` and type `{}` are both `{}` in WIT: rename one", names[at], m.name));
            e.combine(Error::new(names[at].span(), format!("type `{}` is here", names[at])));
            return Err(e);
        }
        let companion = m.watch.as_ref().map(|watch| {
            let mut companion = m.clone();
            companion.name = watch.stream.clone();
            companion.watch = None;
            let Returns::One(item) = &m.returns else { unreachable!() };
            companion.returns = Returns::Stream(item.clone());
            companion.waits = true;
            companion
        });
        ir.methods.push(m);
        ir.methods.extend(companion);
    }
    for m in ir.methods.iter().filter(|m| m.watch.is_some()) {
        let stream = &m.watch.as_ref().unwrap().stream;
        if ir.methods.iter().filter(|other| &other.name == stream).count() != 1
            || ir.children.iter().any(|child| &child.name == stream) || ir.types.iter().any(|ty| &ty.name == stream) {
            return err(&module, &format!("generated watch method {stream} collides with another interface name"));
        }
    }
    if let Some((role, held, why)) = ir.unknown_in_roles() {
        let at = role_spans.iter().find(|(r, n, _)| *r == role.name && n == held).map(|(.., s)| *s).unwrap_or_else(|| module.span());
        return Err(Error::new(at, why));
    }

    let (wit, json, ts) = (plexus_gen::wit(&ir), ir.json(), plexus_gen::ts(&ir));
    let (vis, ident, attrs) = (&module.vis, &module.ident, &module.attrs);
    let hash = ir.hash();
    let consts = quote! {
        /// This activation's IR, as JSON.
        pub const IR: &str = #json;
        /// Its interface hash: what a server says at hello.
        pub const HASH: &str = #hash;
        /// Its WIT, written from the IR.
        pub const WIT: &str = #wit;
        /// Its typed client and its activation side, in TypeScript, written from the IR.
        pub const CLIENT_TS: &str = #ts;
    };
    if describe {
        let items = plain(items).into_iter().map(|i| i.into_token_stream());
        return Ok((ir, quote! { #(#attrs)* #vis mod #ident { #(#items)* #consts } }));
    }
    let grants = ir.methods.iter().filter_map(|method| {
        let grant = method.grant.as_deref()?;
        let mut roles = std::collections::BTreeSet::new();
        loop {
            let before = roles.len();
            for role in &ir.roles {
                if role.grants.iter().any(|g| g == grant) || role.includes.iter().any(|r| roles.contains(r.as_str())) {
                    roles.insert(role.name.as_str());
                }
            }
            if roles.len() == before { break; }
        }
        let name = &method.name;
        Some(quote!(server.require_grant(format!("{at}.{}", #name), #grant, &[#(#roles),*]);))
    });
    let has_grants = ir.methods.iter().any(|m| m.grant.is_some());
    let has_input = ir.methods.iter().any(|m| m.input.is_some());
    let has_watches = ir.methods.iter().any(|m| m.watch.is_some());
    let native_only = has_watches || has_encoded || ir.context.is_some() || ir.methods.iter().any(|m| m.caller.is_some()) || has_input || has_grants;
    let native_module = native_only.then(|| quote!(#[cfg(not(target_arch = "wasm32"))]));
    let native_error = native_only.then(|| {
        let message = if has_watches {
            "watchable reads require a native activation"
        } else if has_input {
            "Rust component input streams are not served yet: build this activation natively, or use #[activation(describe)]"
        } else if has_encoded {
            "Encoded<T> returns require a native activation"
        } else if has_grants {
            "method grants require a native server that enforces them"
        } else {
            "activations using a caller or context must be built natively"
        };
        let e = Error::new(module.ident.span(), message).to_compile_error();
        quote!(#[cfg(target_arch = "wasm32")] const _: () = { #e };)
    });
    // Natively a child is served; a component holds none yet.
    let no_component = (!ir.children.is_empty()).then(|| {
        let e = Error::new(module.ident.span(), "children are not in a component yet: build this activation natively, or use #[activation(describe)]").to_compile_error();
        quote!(#[cfg(target_arch = "wasm32")] const _: () = { #e };)
    });
    let world = format!("{}-activation", ir.activation);
    // Each field of the activation's struct is offered to the runtime when it
    // is served: a `Store` among them is claimed by the place, so its
    // database is the activation's own (plexus's `store`).
    let fields = items.iter().find_map(|i| match i {
        Item::Struct(s) if s.ident == activation => Some(&s.fields),
        _ => None,
    });
    let members: Vec<syn::Member> = fields.into_iter().flatten().enumerate().map(|(n, f)| match &f.ident {
        Some(name) => syn::Member::Named(name.clone()),
        None => syn::Member::Unnamed(n.into()),
    }).collect();
    let claims = (!members.is_empty()).then(|| quote! {
        {
            use ::plexus::__private::{ClaimNone as _, ClaimStore as _};
            #((&self.#members).__plexus_claim(at);)*
        }
    });
    // Bootstrap may mount descendants after serving this activation.
    let descendant_guard = context.as_ref().map(|context| guard_descendants(&ir, context, &activation));
    let kept = plain(&kept);
    let subscription_impl = (!subscriptions.is_empty()).then(|| quote! {
        #[cfg(not(target_arch = "wasm32"))]
        impl #activation { #(#subscriptions)* }
    });
    let native_types = data.iter().zip(&ir.types).map(|(item, def)| native_type(&ir, item, def));
    // One file, two builds: to wasm, a component exporting the WIT, its
    // data types wit-bindgen's; natively, the data types as written, read
    // and written by the value layout, and `serve`.
    let tokens = quote! {
        #native_error
        #native_module
        #(#attrs)*
        #vis mod #ident {
            #[cfg(target_arch = "wasm32")]
            ::wit_bindgen::generate!({ inline: #wit, world: #world });
            #[cfg(target_arch = "wasm32")]
            pub use #exports::{#(#names),*};
            #(#native_types)*
            #consts
            #(#kept)*
            #subscription_impl

            #[cfg(target_arch = "wasm32")]
            ::std::thread_local! {
                static __ACTIVATION: ::std::rc::Rc<#activation> = ::std::rc::Rc::new(<#activation as ::std::default::Default>::default());
            }
            #[cfg(target_arch = "wasm32")]
            struct __Component;
            #[cfg(target_arch = "wasm32")]
            impl #exports::Guest for __Component {
                #(#glue)*
            }
            #[cfg(target_arch = "wasm32")]
            export!(__Component);

            #no_component

            /// Serve `activation` on `server` at `at` (its name, unless a
            /// parent holds it): every method, its IR and its hash, and every
            /// child under `<at>.<child>`. The activation is moved to a
            /// thread of its own, where every call runs, as in a component
            /// (`plexus::__private::Local`); a child held by reference runs
            /// there too.
            #[cfg(not(target_arch = "wasm32"))]
            pub fn serve(activation: #activation, server: &mut ::plexus::Server, at: &str) {
                let (__plexus_local, __plexus_later) = ::plexus::__private::Local::later();
                server.hello(HASH);
                <#activation as ::plexus::Serve>::held(&activation, &__plexus_local, |__plexus_a| __plexus_a, server, at);
                __plexus_later.start(activation);
            }
            #[cfg(not(target_arch = "wasm32"))]
            impl ::plexus::Serve for #activation {
                const NAME: &'static str = #name;
                fn serve(self, server: &mut ::plexus::Server, at: &str) {
                    serve(self, server, at)
                }
                fn held<__P, __G>(&self, __plexus_local: &::plexus::__private::Local<__P>, __plexus_get: __G, server: &mut ::plexus::Server, at: &str)
                where
                    __P: ::std::marker::Send + 'static,
                    __G: for<'__a> ::std::ops::Fn(&'__a __P) -> &'__a Self + ::std::marker::Copy + ::std::marker::Send + ::std::marker::Sync + 'static,
                {
                    #claims
                    server.describe(at, IR).expect("the IR says its hash");
                    #(#served)*
                    #(#grants)*
                    #descendant_guard
                    #(#mounted)*
                }
            }
        }
    };
    Ok((ir, tokens))
}

/// A type as the IR has it, through any aliases.
fn resolved<'a>(ir: &'a Ir, t: &'a Ty) -> &'a Ty {
    match t {
        Ty::Named(n) => match ir.types.iter().find(|d| d.name == *n).map(|d| &d.shape) {
            Some(Shape::Alias(t, _)) => resolved(ir, t),
            _ => t,
        },
        t => t,
    }
}

fn is_bytes(ir: &Ir, t: &Ty) -> bool {
    *resolved(ir, t) == Ty::Prim("bytes")
}

/// Bytes inside a list, an option, a map or a tuple: the value layout
/// writes them as base64, which serde's own does not, so they are not
/// served natively yet.
fn holds_bytes(ir: &Ir, t: &Ty) -> bool {
    match resolved(ir, t) {
        Ty::List(t) | Ty::Option(t) | Ty::Map(t) => is_bytes(ir, t) || holds_bytes(ir, t),
        Ty::Tuple(ts) => ts.iter().any(|t| is_bytes(ir, t) || holds_bytes(ir, t)),
        _ => false,
    }
}

fn not_served_yet(at: impl Spanned) -> proc_macro2::TokenStream {
    let e = Error::new(at.span(), "bytes inside a list, an option, a map or a tuple are not served natively yet: make them a field of a record").to_compile_error();
    quote!(#[cfg(not(target_arch = "wasm32"))] const _: () = { #e };)
}

/// The serde attributes of one field: its wire name, an option left out
/// when none, bytes as base64.
fn serde_field(ir: &Ir, wire: &str, t: &Ty) -> proc_macro2::TokenStream {
    let mut out = quote!(#[serde(rename = #wire)]);
    if matches!(t, Ty::Option(_)) {
        out.extend(quote!(#[serde(default, skip_serializing_if = "::std::option::Option::is_none")]));
    }
    if is_bytes(ir, t) {
        out.extend(quote!(#[serde(with = "::plexus::__private::bytes")]));
    }
    out
}

/// Each field's serde attributes, beside its IR's.
fn field_attrs(ir: &Ir, fields: &mut Fields, irs: &[Field], refused: &mut Vec<proc_macro2::TokenStream>) {
    for (f, x) in fields.iter_mut().zip(irs) {
        let t = x.ty.as_ref().expect("a field has a type");
        if holds_bytes(ir, t) {
            refused.push(not_served_yet(&f.ty));
        }
        let attrs = serde_field(ir, &x.name, t);
        f.attrs.extend(syn::parse::Parser::parse2(Attribute::parse_outer, attrs).expect("attributes"));
    }
}

/// A data type as written, for a native build: wit-bindgen's derives (the
/// component's own type has them), and serde's, by the value layout.
fn native_type(ir: &Ir, item: &Item, def: &TypeDef) -> proc_macro2::TokenStream {
    let mut item = plain(std::slice::from_ref(item)).remove(0);
    let derive = quote!(#[derive(::std::clone::Clone, ::std::fmt::Debug, ::plexus::__private::serde::Serialize, ::plexus::__private::serde::Deserialize)]);
    let krate = quote!(#[serde(crate = "::plexus::__private::serde")]);
    let mut refused = Vec::new();
    let extra = match (&mut item, &def.shape) {
        (Item::Struct(s), Shape::Record(fields)) => {
            s.attrs.retain(|a| !a.path().is_ident("derive"));
            field_attrs(ir, &mut s.fields, fields, &mut refused);
            quote!()
        }
        (Item::Enum(e), Shape::Enum(cases) | Shape::Variant(cases)) => {
            e.attrs.retain(|a| !a.path().is_ident("derive"));
            for (v, c) in e.variants.iter_mut().zip(cases) {
                let wire = c.wire.as_deref().unwrap_or(&c.name);
                v.attrs.push(syn::parse_quote!(#[serde(rename = #wire)]));
                match (&mut v.fields, &c.ty) {
                    (Fields::Unnamed(u), Some(t)) => {
                        if holds_bytes(ir, t) {
                            refused.push(not_served_yet(&u.unnamed[0].ty));
                        } else if is_bytes(ir, t) {
                            u.unnamed[0].attrs.push(syn::parse_quote!(#[serde(with = "::plexus::__private::bytes")]));
                        }
                    }
                    (fields @ Fields::Named(_), None) => field_attrs(ir, fields, c.record.as_deref().unwrap_or_default(), &mut refused),
                    _ => {}
                }
            }
            if matches!(def.shape, Shape::Enum(_)) {
                quote!(#[derive(::std::marker::Copy, ::std::cmp::PartialEq, ::std::cmp::Eq, ::std::cmp::PartialOrd, ::std::cmp::Ord)])
            } else {
                quote!()
            }
        }
        // An alias is itself.
        _ => return quote!(#[cfg(not(target_arch = "wasm32"))] #item),
    };
    quote!(#(#refused)* #[cfg(not(target_arch = "wasm32"))] #derive #extra #krate #item)
}

/// One child's serving, for a native build, under `<at>.<child>`. One
/// held by reference (`&Kiln`, an activation written with `#[activation]`
/// here or in another crate) is served in this process, on its parent's
/// thread, found there through its parent. One that is anything
/// (`Box<dyn Activation>`) or fits an interface (`Box<dyn Glazes>`, where
/// `trait Glazes: plexus::Activation`) is asked for once, when the parent
/// is served, and mounted: every call under its place is passed to it.
fn mounted_child(child: &Child, f: &syn::ImplItemFn, held: &Type) -> Res<proc_macro2::TokenStream> {
    let (method, name) = (&f.sig.ident, &child.name);
    if child.key.is_some() || child.list.is_some() || child.search.is_some() {
        return err(&f.sig, "a child per key is described, not served yet: use #[activation(describe)]");
    }
    if child.fails.is_some() {
        return err(&f.sig.output, "a child that may fail to be there is described, not served yet: return the child itself, or use #[activation(describe)]");
    }
    let at = quote!(&::std::format!("{}.{}", at, #name));
    Ok(match child.held {
        Held::Fixed => {
            let Type::Reference(r) = held else {
                return err(held, "a child held in this process is returned by reference: `&Kiln`");
            };
            let ty = &r.elem;
            quote! {
                <#ty as ::plexus::Serve>::held(self.#method(), __plexus_local, move |__plexus_p: &__P| __plexus_get(__plexus_p).#method(), server, #at);
            }
        }
        Held::Interface | Held::Any => quote! {
            server.mount(#at, { let __plexus_c: ::std::boxed::Box<dyn ::plexus::Activation> = self.#method(); __plexus_c });
        },
    })
}

/// The error translation shared by an open check and a method body.
fn typed_failure(ir: &Ir, ty: &Ty) -> proc_macro2::TokenStream {
    let cases = match resolved(ir, ty) {
        Ty::Named(name) => ir.types.iter().find(|d| d.name == *name).map(|d| &d.shape),
        _ => None,
    };
    let cases: Vec<_> = match cases {
        Some(Shape::Enum(cs) | Shape::Variant(cs)) => cs.iter().map(|c| {
            let (wire, name, doc) = (c.wire.as_deref().unwrap_or(&c.name), &c.name, &c.doc);
            quote!((#wire, #name, #doc))
        }).collect(),
        _ => Vec::new(),
    };
    quote!(::plexus::__private::failed(&__plexus_e, &[#(#cases),*]))
}

fn context_ty(context: &ActivationContext<'_>, activation: &syn::Ident) -> proc_macro2::TokenStream {
    let name = context.name;
    quote!(<#name as ::plexus::Context<#activation>>)
}

/// Resolve the context and check the gate on the thread owning the activation.
fn make_context(ir: &Ir, context: &ActivationContext<'_>, ty: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let open = context.open.zip(context.ir.fails.as_ref()).map(|(f, error)| {
        let method = &f.sig.ident;
        let failure = typed_failure(ir, error);
        quote! {
            __plexus_this.#method(&__plexus_context).map_err(|__plexus_e| #failure)?;
        }
    });
    quote! {
        let __plexus_this = __plexus_get(&*__plexus_a);
        #ty::from_context(__plexus_this, __plexus_call.caller(), __plexus_call.keys())
            .and_then(|__plexus_context| {
                #open
                ::std::result::Result::Ok(__plexus_context)
            })
    }
}

fn guard_descendants(ir: &Ir, context: &ActivationContext<'_>, activation: &syn::Ident) -> proc_macro2::TokenStream {
    let ty = context_ty(context, activation);
    let make = make_context(ir, context, &ty);
    let required = context.requires_caller.then(|| quote!(__plexus_call.require_caller()?;));
    quote! {
        {
            let __plexus_local = __plexus_local.clone();
            server.guard_descendants(at, #ty::KEYS, move |__plexus_call| {
                __plexus_local.validate(__plexus_call, move |__plexus_a: ::std::rc::Rc<__P>, __plexus_call| {
                    #required
                    { #make }.map(|_| ())
                })
            });
        }
    }
}

/// One method's registration on a `plexus::Server`, for a native build:
/// its body read by the value layout (a wrong one fails `invalid-params`,
/// naming the parameter, and the method does not run), the call made on
/// the activation's thread, and its answer, items or typed failure written.
fn served_method(
    ir: &Ir,
    m: &Method,
    f: &syn::ImplItemFn,
    args: &MethodArgs,
    context: Option<&ActivationContext<'_>>,
    activation: &syn::Ident,
    typed_sources: bool,
) -> proc_macro2::TokenStream {
    let MethodArgs { wire_names: idents, wire_types: tys, invocation: call_args, .. } = args;
    let (method, path) = (&f.sig.ident, format!(".{}", m.name));
    let wires: Vec<&str> = m.params.iter().map(|p| p.name.as_str()).collect();
    let mut refused = Vec::new();
    let takes = m.params.iter().zip(tys).map(|(p, ty)| {
        let (wire, t) = (&p.name, p.ty.as_ref().expect("a parameter has a type"));
        if holds_bytes(ir, t) {
            refused.push(not_served_yet(ty));
        }
        let default = match &p.default {
            Some(d) => {
                let d = d.to_string();
                quote!(::std::option::Option::Some(#d))
            }
            None => quote!(::std::option::Option::None),
        };
        if is_bytes(ir, t) { quote!(__plexus_p.take::<::plexus::__private::B64>(#wire, #default)?.0) } else { quote!(__plexus_p.take::<#ty>(#wire, #default)?) }
    }).collect::<Vec<_>>();
    let encoded = encoded_output(&f.sig.output);
    let write = |t: &Ty, v: proc_macro2::TokenStream| {
        if encoded {
            if is_bytes(ir, t) || holds_bytes(ir, t) {
                return Error::new(f.sig.output.span(), "a top-level encoded byte result requires the Layers base64 codec; return the byte value directly").to_compile_error();
            }
            quote!(::std::result::Result::Ok(::plexus::Encoded::into_raw(#v)))
        } else if is_bytes(ir, t) {
            quote!(::plexus::raw(&::plexus::__private::B64(#v)))
        } else {
            quote!(::plexus::raw(&#v))
        }
    };
    let args = quote! {
        let __plexus_args = ::plexus::__private::params(&__plexus_body, &[#(#wires),*])
            .and_then(|__plexus_p| ::std::result::Result::Ok((#(#takes,)*)));
    };
    let wait = f.sig.asyncness.is_some().then(|| quote!(.await));
    let call = quote!(__plexus_get(&*__plexus_a).#method(#(#call_args),*) #wait);
    let path = quote!(&::std::format!("{}{}", at, #path));
    let input = m.input.as_ref().map(|t| {
        if holds_bytes(ir, t) {
            refused.push(not_served_yet(&f.sig));
        }
        if is_bytes(ir, t) {
            quote!(__plexus_call.take_byte_input())
        } else {
            quote!(__plexus_call.take_input(::plexus::__private::decode_input))
        }
    });
    let needs_call = context.is_some() || m.caller.is_some() || input.is_some();
    let handler_args = if input.is_some() {
        quote!(__plexus_body: ::plexus::Raw, mut __plexus_call)
    } else if needs_call {
        quote!(__plexus_body: ::plexus::Raw, __plexus_call)
    } else {
        quote!(__plexus_body: ::plexus::Raw)
    };
    let context_ty = context.map(|c| context_ty(c, activation));
    let require_caller = (m.caller == Some(Caller::Required)
        || context.is_some_and(|c| c.requires_caller))
        .then(|| quote!(__plexus_call.require_caller()));
    let make_context = context.zip(context_ty.as_ref()).map(|(c, ty)| make_context(ir, c, ty));
    let tokens = if let Some((item, iterator)) = stream_of(&f.sig.output) {
        let open = if let Some(error) = &m.fails {
            let failure = typed_failure(ir, error);
            quote!(match #call {
                Ok(items) => items,
                Err(__plexus_e) => return ::plexus::__private::fail_stream(__plexus_asks, #failure),
            })
        } else { call };
        let items = if iterator { quote!(::plexus::__private::futures::stream::iter(#open)) } else { open };
        let each = match &m.returns {
            Returns::Stream(t) => write(t, quote!(__plexus_item)),
            _ => unreachable!("a stream returns a stream"),
        };
        let (register, scope) = if let Some(ty) = &context_ty {
            (quote!(::plexus::__private::stream_with_context), quote!(at, #ty::KEYS,))
        } else if needs_call {
            (quote!(::plexus::__private::stream_with_caller), quote!())
        } else {
            (quote!(::plexus::__private::stream), quote!())
        };
        let reject = |name: proc_macro2::TokenStream, make: proc_macro2::TokenStream| quote! {
            let #name = match { #make } {
                ::std::result::Result::Ok(value) => value,
                ::std::result::Result::Err(failure) => {
                    return ::plexus::__private::fail_stream(__plexus_asks, failure);
                }
            };
        };
        let input = input.map(|make| reject(quote!(__plexus_input), make));
        let required = require_caller.map(|make| reject(quote!(__plexus_required_caller), make));
        let context = make_context.map(|make| reject(quote!(__plexus_context), make));
        let handler = quote! {
            move |#handler_args| {
                #args
                __plexus_local.stream(__plexus_args, move |__plexus_a: ::std::rc::Rc<__P>, (#(#idents,)*), __plexus_asks| async move {
                    #input
                    #required
                    #context
                    let __plexus_items = #items;
                    ::plexus::__private::feed(__plexus_asks, __plexus_items, |__plexus_item: #item| #each).await
                })
            }
        };
        quote! {
            let __plexus_local = __plexus_local.clone();
            #register(server, #path, #scope #handler);
        }
    } else {
        let ok = match &m.returns {
            Returns::One(t) => write(t, quote!(__plexus_v)),
            _ => quote!(::plexus::raw(&__plexus_v)),
        };
        let make_answer = |ok: proc_macro2::TokenStream| match &m.fails {
            None => quote!({ let __plexus_v = #call; #ok }),
            Some(e) => {
                let failure = typed_failure(ir, e);
                quote! {
                    match #call {
                        ::std::result::Result::Ok(__plexus_v) => #ok,
                        ::std::result::Result::Err(__plexus_e) => ::std::result::Result::Err(#failure),
                    }
                }
            }
        };
        let answer = make_answer(ok.clone());
        let (register, scope) = if let Some(ty) = &context_ty {
            (quote!(method_with_context), quote!(at, #ty::KEYS,))
        } else if needs_call {
            (quote!(method_with_caller), quote!())
        } else {
            (quote!(method), quote!())
        };
        let input = input.map(|make| quote!(let __plexus_input = #make?;));
        let required = require_caller.map(|make| quote!(let __plexus_required_caller = #make?;));
        let context = make_context.map(|make| quote!(let __plexus_context = { #make }?;));
        let watch = m.watch.as_ref().map(|watch| {
            let deps = &watch.on;
            let companion = &watch.stream;
            let pair = if encoded {
                quote!(Ok(__plexus_v.into_watch_value()))
            } else {
                quote!({ let body = (#ok)?; let digest = ::plexus::watch_digest(&body); Ok((body, digest)) })
            };
            let refresh = make_answer(pair);
            let (source_args, bindings, subscribe) = if typed_sources {
                let helper = watch_sources::helper_name(method);
                (quote!(__plexus_args), quote!((#(#idents,)*)), quote!(__plexus_get(&*__plexus_a).#helper(#(#call_args),*)))
            } else {
                (quote!(__plexus_args.map(|_| ())), quote!(()), quote!(::plexus::WatchSource::subscribe(__plexus_get(&*__plexus_a), &[#(#deps),*], &__plexus_call, &__plexus_body)))
            };
            quote! {
                let __subscribe = __plexus_local.clone();
                let __refresh = __plexus_local.clone();
                server.watch_method(#path, &format!("{at}.{}", #companion),
                    move |__plexus_body, __plexus_call| {
                        #args
                        __subscribe.submit(#source_args, move |__plexus_a: ::std::rc::Rc<__P>, #bindings| async move {
                            #required
                            #context
                            #subscribe
                        })
                    },
                    move |__plexus_body, __plexus_call| {
                        #args
                        __refresh.submit(__plexus_args, move |__plexus_a: ::std::rc::Rc<__P>, (#(#idents,)*)| async move {
                            #required
                            #context
                            #refresh
                        })
                    }
                );
            }
        });
        quote! {
            let __unary = __plexus_local.clone();
            server.#register(#path, #scope move |#handler_args| {
                #args
                __unary.call(__plexus_args, move |__plexus_a: ::std::rc::Rc<__P>, (#(#idents,)*)| async move {
                    #input
                    #required
                    #context
                    #answer
                })
            });
            #watch
        }
    };
    let accepts_input = m.input.as_ref().map(|_| quote!(server.accepts_input(#path);));
    quote!(#(#refused)* { #tokens #accepts_input })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_watch_preserves_contract_and_rejects_name_collisions() {
        let module: ItemMod = syn::parse_quote! {
            mod board {
                struct Board;
                impl Board {
                    #[watch(on = ["rows", "membership"])]
                    pub fn list(&self, caller: &Caller, #[target] group: String) -> Result<u32, String> { todo!() }
                }
            }
        };
        let (ir, _) = expand(quote!(roles(reader = ["list"])), module.clone()).unwrap();
        assert_eq!(Ir::read(&ir.json()).unwrap(), ir);
        let (read, stream) = (&ir.methods[0], &ir.methods[1]);
        assert_eq!(read.watch.as_ref().unwrap().stream, stream.name);
        assert_eq!(read.watch.as_ref().unwrap().on, ["rows", "membership"]);
        assert_eq!(read.grant, stream.grant);
        assert_eq!(read.target, stream.target);
        assert_eq!(read.fails, stream.fails);
        assert_eq!(read.params, stream.params);
        assert_eq!(read.caller, stream.caller);
        let mut collision = module;
        let items=&mut collision.content.as_mut().unwrap().1;
        let Item::Impl(imp)=&mut items[1] else {unreachable!()};
        imp.items.push(syn::parse_quote!(pub fn list_watch(&self) -> u32 { 0 }));
        assert!(expand(quote!(), collision).unwrap_err().to_string().contains("collides"));
    }

    #[test]
    fn encoded_results_preserve_the_interface_and_dispatch_through_the_typed_wrapper() {
        let module = |result: proc_macro2::TokenStream| -> ItemMod {
            syn::parse2(quote! {
                mod tickets {
                    pub struct Page { pub rows: Vec<String> }
                    pub enum Denied { Missing }
                    pub struct Tickets;
                    impl Tickets {
                        pub fn list(&self) -> #result { todo!() }
                        pub fn checked(&self) -> Result<#result, Denied> { todo!() }
                        pub fn watch(&self) -> impl Stream<Item = #result> { todo!() }
                    }
                }
            }).unwrap()
        };
        let (before, _) = expand(quote!(), module(quote!(Page))).unwrap();
        let (after, tokens) = expand(quote!(), module(quote!(plexus::Encoded<Page>))).unwrap();
        assert_eq!(before.json(), after.json());
        assert_eq!(plexus_gen::ts(&before), plexus_gen::ts(&after));
        assert_eq!(plexus_gen::wit(&before), plexus_gen::wit(&after));
        assert_eq!(tokens.to_string().matches("Encoded :: into_raw").count(), 3);
        let (_, invalid) = expand(quote!(), module(quote!(plexus::Encoded<Vec<u8>>))).unwrap();
        assert!(invalid.to_string().contains("requires the Layers base64 codec"));
    }

    /// The IR of the first `#[activation]` module in a file.
    fn ir_of_file(path: &std::path::Path) -> Ir {
        let file = syn::parse_file(&std::fs::read_to_string(path).expect("the file")).expect("Rust");
        let mut module = file.items.into_iter().find_map(|i| if let Item::Mod(m) = i { Some(m) } else { None }).expect("a module");
        let at = module.attrs.iter().position(|a| a.path().segments.last().is_some_and(|s| s.ident == "activation")).expect("#[activation]");
        let args = match module.attrs.remove(at).meta { Meta::List(l) => l.tokens, _ => Default::default() };
        expand(args, module).expect("an activation").0
    }

    /// `EMIT=ir|wit|ts ACTIVATION_FILE=path/to/lib.rs cargo test -p plexus-macros -- --nocapture emit`
    /// prints that form of the first `#[activation]` module in the file.
    #[test]
    fn emit_for_a_file() {
        let (Ok(path), Ok(what)) = (std::env::var("ACTIVATION_FILE"), std::env::var("EMIT")) else { return };
        let ir = ir_of_file(path.as_ref());
        let text = match what.as_str() {
            "wit" => plexus_gen::wit(&ir),
            "ts" => plexus_gen::ts(&ir),
            _ => ir.json(),
        };
        println!("-----8<-----\n{text}-----8<-----");
    }

    fn examples() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plexus-examples")
    }

    /// Each example by the name its goldens are kept under, and its source.
    fn sources() -> Vec<(String, std::path::PathBuf)> {
        let examples = examples();
        let mut sources: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(examples.join("showcase/src"))
            .expect("plexus-examples beside this checkout")
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_stem().is_some_and(|s| s != "lib"))
            .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
            .collect();
        sources.extend(["kiln", "glaze"].map(|c| (c.to_string(), examples.join(c).join("src/lib.rs"))));
        sources.sort();
        sources
    }

    /// `CLIENT_TS` is the TypeScript `plexus-gen ts` writes from the kept IR
    /// (`ts/` in plexus-examples, plexus-gen's goldens), and `WIT` its WIT.
    #[test]
    fn the_text_is_plexus_gens() {
        for (name, path) in sources() {
            let ir = ir_of_file(&path);
            let ts = std::fs::read_to_string(examples().join(format!("ts/{name}.plexus.ts"))).unwrap();
            assert!(plexus_gen::ts(&ir) == ts, "{name}: CLIENT_TS is not ts/{name}.plexus.ts");
            let wit = std::fs::read_to_string(examples().join(format!("wit/{name}.wit"))).unwrap();
            assert!(plexus_gen::wit(&ir) == wit, "{name}: WIT is not wit/{name}.wit");
        }
    }

    /// Every example in `../plexus-examples` writes the IR kept for it in
    /// its `ir/`: a showcase file as `ir/<file>.json`, kiln and glaze as
    /// `ir/kiln.json` and `ir/glaze.json`. `BLESS=1 cargo test goldens`
    /// writes them anew.
    #[test]
    fn goldens() {
        let (examples, sources) = (examples(), sources());
        let bless = std::env::var_os("BLESS").is_some();
        let mut wrong = Vec::new();
        for (name, path) in &sources {
            let (made, golden) = (ir_of_file(path).json(), examples.join(format!("ir/{name}.json")));
            if bless {
                std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
                std::fs::write(&golden, &made).unwrap();
                continue;
            }
            let kept = std::fs::read_to_string(&golden).unwrap_or_default();
            if kept != made {
                let diff = similar::TextDiff::from_lines(&kept, &made).unified_diff().header(&format!("ir/{name}.json (kept)"), "what the macro writes").to_string();
                wrong.push(diff);
            }
        }
        let mut kept: Vec<String> = std::fs::read_dir(examples.join("ir")).unwrap().map(|e| e.unwrap().path().file_stem().unwrap().to_string_lossy().into_owned()).collect();
        kept.sort();
        let names: Vec<String> = sources.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(kept, names, "a golden for each example, and none for what is not one");
        assert!(wrong.is_empty(), "the IR drifted from its golden (BLESS=1 cargo test goldens keeps what is written now):\n{}", wrong.join("\n"));
    }

    fn shop() -> (Ir, String) {
        let module: ItemMod = syn::parse_quote! {
            /// A shop.
            mod shop {
                pub struct Item { pub name: String, pub in_stock: u32 }
                pub enum ShopError { SoldOut, NoSuch(String) }
                pub enum Size { Small, Large }
                #[derive(Default)]
                pub struct Shop;
                impl Shop {
                    pub fn items(&self) -> Vec<Item> { vec![] }
                    pub async fn buy(&self, name: String, count: u32) -> Result<Item, ShopError> { todo!() }
                    pub fn close(&self) {}
                    pub fn prices(&self, upto: u32) -> impl Stream<Item = f32> { futures::stream::iter((0..upto).map(|n| n as f32)) }
                    fn private(&self) {}
                }
            }
        };
        let (ir, tokens) = expand(Default::default(), module).unwrap();
        (ir, tokens.to_string())
    }

    #[test]
    fn the_wit() {
        let wit = plexus_gen::wit(&shop().0);
        assert!(wit.contains("  record item {\n    name: string,\n    in-stock: u32,\n  }"), "{wit}");
        assert!(wit.contains("  variant shop-error {\n    sold-out,\n    no-such(string),\n  }"), "{wit}");
        assert!(wit.contains("  enum size {\n    small,\n    large,\n  }"), "{wit}");
        assert!(wit.contains("  items: func() -> list<item>;"), "{wit}");
        assert!(wit.contains("  buy: async func(name: string, count: u32) -> result<item, shop-error>;"), "{wit}");
        assert!(wit.contains("  close: func();"), "{wit}");
        assert!(wit.contains("  prices: async func(upto: u32) -> stream<f32>;"), "{wit}");
        assert!(!wit.contains("private"), "{wit}");
    }

    #[test]
    fn the_ir() {
        let json = shop().0.json();
        assert!(json.contains(r#""activation": "shop""#), "{json}");
        assert!(json.contains(r#""name": "buy""#) && json.contains(r#""fails": "shop-error""#), "{json}");
        assert_eq!(Ir::read(&json).unwrap().json(), json);
        assert!(json.contains(r#""stream": "f32""#), "{json}");
        assert!(json.contains(r#""returns": {"list": "item"}"#), "{json}");
    }

    #[test]
    fn a_component_refuses_mut_self() {
        let module: ItemMod = syn::parse_quote! {
            mod shop { #[derive(Default)] pub struct Shop; impl Shop { pub fn close(&mut self) {} } }
        };
        assert!(expand(Default::default(), module).unwrap_err().to_string().contains("&self"));
    }

    #[test]
    fn a_wire_name() {
        let module: ItemMod = syn::parse_quote! {
            mod tickets {
                pub enum Status { Draft, #[wire("In Review")] InReview }
                pub struct Tickets;
                impl Tickets { pub fn current(&self) -> Status { todo!() } }
            }
        };
        let (ir, tokens) = expand(quote!(describe), module).unwrap();
        let json = ir.json_line();
        assert!(json.contains(r#"[{"name": "draft"}, {"name": "in-review", "wire": "In Review"}]"#), "{json}");
        assert!(!tokens.to_string().contains("# [wire"), "{tokens}");
        let ts = plexus_gen::ts(&ir);
        assert!(ts.contains(r#"const toStatus = (v: Status): any => (({ "in-review": "In Review" } as any)[v] ?? v);"#), "{ts}");
    }

    #[test]
    fn described_everything() {
        let module: ItemMod = syn::parse_quote! {
            mod tickets {
                #[limit(min_len = 1, max_len = 64, pattern = "^[A-Z]+-[0-9]+$")]
                pub type TicketId = String;
                pub struct Ticket { pub id: TicketId, #[default("Draft")] pub status: String, pub labels: BTreeMap<String, Json> }
                pub enum Change { Created { id: TicketId, title: String }, Deleted(TicketId), Cleared }
                pub struct Tickets;
                impl Tickets {
                    pub fn close(&self, #[target] id: TicketId, #[default(false)] force: bool, caller: &Caller) -> Result<Ticket, String> { todo!() }
                    pub fn attach(&self, id: TicketId, file: Input<Bytes>, progress: Progress<u8>) -> u64 { todo!() }
                    pub fn whoami(&self, caller: Option<&Caller>) -> String { todo!() }
                    #[child]
                    pub fn commits(&self) -> &Commits { todo!() }
                    #[child(list = "projects")]
                    pub fn project(&self, name: &str) -> Result<Project, NotFound> { todo!() }
                }
            }
        };
        let args = quote!(describe, roles(viewer = ["whoami"], editor = [viewer, "close", "attach"]));
        let (ir, tokens) = expand(args, module).unwrap();
        let json = ir.json_line();
        for want in [
            r#""alias": "string", "limits": {"min-len": 1, "max-len": 64, "pattern": "^[A-Z]+-[0-9]+$"}"#,
            r#""default": "Draft""#,
            r#"{"map": "json"}"#,
            r#""name": "created", "record": ["#,
            r#""caller": "required""#,
            r#""caller": "optional""#,
            r#""grant": "tickets.close""#,
            r#""target": "id""#,
            r#""input": "bytes""#,
            r#""progress": "u8""#,
            r#"{"name": "editor", "includes": ["viewer"], "grants": ["tickets.close", "tickets.attach"]}"#,
            r#""name": "commits", "activation": "commits""#,
            r#""list": "projects""#,
        ] {
            assert!(json.contains(want), "missing {want}\n{json}");
        }
        assert!(!json.contains(r#""name": "caller""#) && !json.contains(r#""name": "file""#), "the caller and the input are no parameters\n{json}");
        let rust = tokens.to_string();
        assert!(!rust.contains("# [target]") && !rust.contains("# [child") && !rust.contains("# [limit"), "{rust}");
        assert!(plexus_gen::wit(&ir).contains("type ticket-id = string;"), "{}", plexus_gen::wit(&ir));
    }

    #[test]
    fn input_stream_is_injected_separately_from_ordinary_params() {
        let module: ItemMod = syn::parse_quote! {
            mod media {
                pub struct Media;
                impl Media {
                    pub async fn put(&self, data: Input<Bytes>, name: String, caller: &Caller) -> String { todo!() }
                    pub fn words(&self, data: Input<String>) -> impl Stream<Item = String> { todo!() }
                }
            }
        };
        let (ir, generated) = expand(Default::default(), module).unwrap();
        let put = &ir.methods[0];
        assert_eq!(put.input, Some(Ty::Prim("bytes")));
        assert_eq!(put.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["name"]);
        let rust = generated.to_string();
        for want in ["take_input", "take_byte_input",
                     "accepts_input", "put (__plexus_input , name , __plexus_required_caller)", "stream_with_caller"] {
            assert!(rust.contains(want), "missing {want}");
        }
    }

    #[test]
    fn refuses_ambiguous_inputs() {
        for (params, why) in [
            (quote!(a: Input<u8>, b: Input<u8>), "at most one Input"),
            (quote!(a: &Input<u8>), "an owned Input"),
            (quote!(a: Input), "an owned Input"),
        ] {
            let module: ItemMod = syn::parse_quote! {
                mod media { pub struct Media; impl Media { pub fn put(&self, #params) {} } }
            };
            assert!(expand(Default::default(), module).unwrap_err().to_string().contains(why));
        }
    }

    #[test]
    fn byte_alias_input_uses_the_byte_decoder_but_a_record_uses_serde() {
        let module: ItemMod = syn::parse_quote! {
            mod media {
                pub type Chunk = Bytes;
                pub type Blob = Chunk;
                pub struct Photo { pub image: Blob }
                pub struct Media;
                impl Media {
                    pub fn upload(&self, parts: Input<Blob>) { todo!() }
                    pub fn import(&self, photos: Input<Photo>) { todo!() }
                }
            }
        };
        let (_, generated) = expand(Default::default(), module).unwrap();
        let rust = generated.to_string();
        assert_eq!(rust.matches("take_byte_input").count(), 1);
        assert_eq!(rust.matches("take_input (:: plexus :: __private :: decode_input)").count(), 1);
    }

    #[test]
    fn a_role_grants_a_method_that_is_not_there() {
        let module: ItemMod = syn::parse_quote! {
            mod tickets { pub struct Tickets; impl Tickets { pub fn fetch(&self) -> String { todo!() } } }
        };
        let e = expand(quote!(describe, roles(viewer = ["get"])), module.clone()).unwrap_err().to_string();
        assert_eq!(e, "role `viewer` grants `tickets.get`, which is no method of `tickets`");
        let e = expand(quote!(describe, roles(editor = [viewer, "fetch"])), module.clone()).unwrap_err().to_string();
        assert_eq!(e, "role `editor` includes `viewer`, which is no role");
        assert!(expand(quote!(describe, roles(viewer = ["fetch"])), module).is_ok());
    }

    #[test]
    fn a_method_and_a_type_of_one_name() {
        let module: ItemMod = syn::parse_quote! {
            mod jobs {
                pub struct Batch { pub size: u32 }
                #[derive(Default)]
                pub struct Jobs;
                impl Jobs { pub fn batch(&self) -> Batch { todo!() } }
            }
        };
        let e = expand(Default::default(), module).unwrap_err();
        let said: Vec<String> = e.into_iter().map(|e| e.to_string()).collect();
        assert_eq!(said, ["method `batch` and type `Batch` are both `batch` in WIT: rename one", "type `Batch` is here"]);
    }

    #[test]
    fn served_natively() {
        let module: ItemMod = syn::parse_quote! {
            mod shop {
                pub struct Item { pub in_stock: u32, pub photo: Bytes, pub note: Option<String> }
                pub enum ShopError { SoldOut, #[wire("No Such")] NoSuch(String) }
                #[derive(Default)]
                pub struct Shop;
                impl Shop {
                    pub async fn buy(&self, name: String, #[default(1)] count: u32) -> Result<Item, ShopError> { todo!() }
                    pub fn prices(&self, upto: u32) -> impl Iterator<Item = f32> { (0..upto).map(|n| n as f32) }
                }
            }
        };
        let rust = expand(Default::default(), module).unwrap().1.to_string();
        for want in [
            r#"# [cfg (not (target_arch = "wasm32"))] pub fn serve (activation : Shop , server : & mut :: plexus :: Server , at : & str)"#,
            r#"impl :: plexus :: Serve for Shop"#,
            r#"# [serde (rename = "in-stock")] pub in_stock"#,
            r#"# [serde (rename = "photo")] # [serde (with = "::plexus::__private::bytes")] pub photo"#,
            r#"# [serde (default , skip_serializing_if = "::std::option::Option::is_none")] pub note"#,
            r#"# [serde (rename = "No Such")] NoSuch"#,
            r#"__plexus_p . take :: < u32 > ("count" , :: std :: option :: Option :: Some ("1")) ?"#,
            r#"& [("sold-out" , "sold-out" , "") , ("No Such" , "no-such" , "")]"#,
            r#":: plexus :: __private :: futures :: stream :: iter"#,
            r#"# [cfg (target_arch = "wasm32")] :: wit_bindgen :: generate !"#,
        ] {
            assert!(rust.contains(want), "missing {want}\n{rust}");
        }
    }

    #[test]
    fn context_describes_only_wire_types_and_keys() {
        let module: ItemMod = syn::parse_quote! {
            mod studio {
                #[derive(plexus::Context)]
                pub struct StudioCx {
                    #[from(caller)]
                    user: User,
                    studio_id: u32,
                }
                struct User(String);
                pub enum Denied { Missing }
                pub struct Studio;
                impl Studio {
                    #[open]
                    pub fn open(&self, cx: &StudioCx) -> Result<(), Denied> { todo!() }
                    pub fn query(&self, cx: &StudioCx, text: String) -> u32 { todo!() }
                    pub fn events(&self) -> impl Iterator<Item = u32> { todo!() }
                }
            }
        };
        let (ir, tokens) = expand(quote!(context = StudioCx), module).unwrap();
        assert_eq!(ir.types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["denied"]);
        let context = ir.context.unwrap();
        assert_eq!(context.keys[0].name, "studio-id");
        assert_eq!(context.keys[0].ty, Some(Ty::Prim("u32")));
        assert_eq!(context.fails, Some(Ty::Named("denied".into())));
        assert_eq!(ir.methods.len(), 2);
        assert!(ir.methods.iter().all(|m| m.caller == Some(Caller::Required)));
        assert_eq!(ir.methods[0].params.len(), 1);
        assert_eq!(ir.methods[0].params[0].name, "text");
        let rust = tokens.to_string();
        assert!(rust.contains("must be built natively"));
        assert_eq!(rust.matches(". open (& __plexus_context)").count(), 3);
        assert!(rust.contains("method_with_context"));
        assert!(rust.contains("stream_with_context"));
        assert!(rust.contains("guard_descendants"));
    }

    #[test]
    fn context_keys_must_have_distinct_wire_names() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct StudioCx { studio_id: u32, studio__id: u32 }
        };
        let e = derive_context(input).unwrap_err().to_string();
        assert_eq!(e, "duplicate context key `studio-id`");
    }

    #[test]
    fn contextual_children_register_an_ancestor_guard() {
        let module: ItemMod = syn::parse_quote! {
            mod studio {
                pub struct StudioCx {
                    #[from(caller)]
                    caller: Caller,
                    studio_id: u32,
                }
                pub enum Denied { Missing }
                pub struct Studio;
                impl Studio {
                    #[open]
                    pub fn open(&self, cx: &StudioCx) -> Result<(), Denied> { todo!() }
                    pub fn status(&self) -> u32 { todo!() }
                    #[child]
                    pub fn kiln(&self) -> &Kiln { todo!() }
                    #[child]
                    pub fn glaze(&self) -> Box<dyn Activation> { todo!() }
                    #[child]
                    pub fn supplier(&self) -> Box<dyn Glazes> { todo!() }
                }
            }
        };
        let (ir, tokens) = expand(quote!(context = StudioCx), module).unwrap();
        assert_eq!(ir.children.len(), 3);
        assert!(ir.children.iter().all(|child| child.key.is_none()));
        assert!(ir.methods[0].params.is_empty());
        let rust = tokens.to_string();
        assert_eq!(rust.matches("server . guard_descendants").count(), 1);
        assert_eq!(rust.matches(". open (& __plexus_context)").count(), 2);
        for want in [
            "must be built natively",
            "guard_descendants (at , < StudioCx as :: plexus :: Context < Studio >> :: KEYS",
            "__plexus_local . validate (__plexus_call",
            "__plexus_call . require_caller () ?",
            "__plexus_call . caller () , __plexus_call . keys ()",
            ":: plexus :: __private :: failed (& __plexus_e",
            "< Kiln as :: plexus :: Serve > :: held",
            "server . mount",
        ] {
            assert!(rust.contains(want), "missing {want}\n{rust}");
        }
    }

    #[test]
    fn mounted_factories_cannot_receive_request_credentials() {
        for argument in [quote!(cx: &StudioCx), quote!(caller: &Caller)] {
            let module: ItemMod = syn::parse_quote! {
                mod studio {
                    pub struct StudioCx { studio_id: u32 }
                    pub struct Studio;
                    impl Studio {
                        #[child]
                        pub fn kiln(&self, #argument) -> &Kiln { todo!() }
                    }
                }
            };
            let e = expand(quote!(context = StudioCx), module).unwrap_err().to_string();
            assert_eq!(e, "a mounted child factory cannot take a context or caller");
        }
    }

    #[test]
    fn children_served_natively() {
        let module: ItemMod = syn::parse_quote! {
            mod studio {
                pub trait Glazes: plexus::Activation {}
                pub struct Studio { kiln: Kiln, glaze: plexus::Remote }
                impl Studio {
                    #[child]
                    pub fn kiln(&self) -> &Kiln { &self.kiln }
                    #[child]
                    pub fn glaze(&self) -> Box<dyn Activation> { todo!() }
                    #[child]
                    pub fn supplier(&self) -> Box<dyn Glazes> { todo!() }
                }
            }
        };
        let rust = expand(Default::default(), module).unwrap().1.to_string();
        for want in [
            r#"< Kiln as :: plexus :: Serve > :: held (self . kiln () , __plexus_local , move | __plexus_p : & __P | __plexus_get (__plexus_p) . kiln () , server , & :: std :: format ! ("{}.{}" , at , "kiln"))"#,
            r#"server . mount (& :: std :: format ! ("{}.{}" , at , "glaze") , { let __plexus_c : :: std :: boxed :: Box < dyn :: plexus :: Activation > = self . glaze () ; __plexus_c })"#,
            r#"server . mount (& :: std :: format ! ("{}.{}" , at , "supplier")"#,
            r#"# [cfg (target_arch = "wasm32")] const _ : () = { :: core :: compile_error ! { "children are not in a component yet"#,
            // every field offered, before anything is registered: a Store among them is claimed by the place
            r#"{ use :: plexus :: __private :: { ClaimNone as _ , ClaimStore as _ } ; (& self . kiln) . __plexus_claim (at) ; (& self . glaze) . __plexus_claim (at) ; } server . describe (at , IR)"#,
        ] {
            assert!(rust.contains(want), "missing {want}\n{rust}");
        }
    }

    #[test]
    fn a_child_per_key_is_described_only() {
        let module: ItemMod = syn::parse_quote! {
            mod studio {
                pub struct Studio;
                impl Studio {
                    #[child(list = "shelves")]
                    pub fn shelf(&self, number: u32) -> Result<Kiln, NotFound> { todo!() }
                    pub fn shelves(&self) -> Vec<u32> { vec![] }
                }
            }
        };
        let e = expand(Default::default(), module.clone()).unwrap_err().to_string();
        assert_eq!(e, "a child per key is described, not served yet: use #[activation(describe)]");
        assert!(expand(quote!(describe), module).is_ok());
        let contextual: ItemMod = syn::parse_quote! {
            mod studio {
                pub struct StudioCx { studio_id: u32 }
                pub struct Studio;
                impl Studio {
                    #[child]
                    pub fn shelf(&self, number: u32) -> &Kiln { todo!() }
                }
            }
        };
        let e = expand(quote!(context = StudioCx), contextual).unwrap_err().to_string();
        assert_eq!(e, "a child per key is described, not served yet: use #[activation(describe)]");
        let owned: ItemMod = syn::parse_quote! {
            mod studio { pub struct Studio; impl Studio { #[child] pub fn kiln(&self) -> Kiln { todo!() } } }
        };
        let e = expand(Default::default(), owned).unwrap_err().to_string();
        assert_eq!(e, "a child held in this process is returned by reference: `&Kiln`");
    }

    #[test]
    fn bytes_inside_an_option_are_refused_natively() {
        let module: ItemMod = syn::parse_quote! {
            mod shop {
                pub struct Item { pub photo: Option<Bytes> }
                #[derive(Default)]
                pub struct Shop;
                impl Shop { pub fn get(&self) -> Item { todo!() } }
            }
        };
        let rust = expand(Default::default(), module).unwrap().1.to_string();
        assert!(rust.contains(r#"# [cfg (not (target_arch = "wasm32"))] const _ : () = { :: core :: compile_error ! { "bytes inside a list"#), "{rust}");
    }
}
