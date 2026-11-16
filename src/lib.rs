//! `#[activation]` on a module: the module's types and its one `impl` block
//! are read into an IR (`plexus-ir`), and from the IR come its WIT and its
//! TypeScript.
//!
//! Two forms:
//!
//! - `#[activation]`: the module also becomes a WebAssembly component that
//!   exports the WIT. The activation is made once, with `Default`, and its
//!   methods take `&self`. An `async fn` is an `async func`; a method that
//!   returns `impl Stream<Item = T>` gives a `stream<T>`. Data types are
//!   replaced by the ones wit-bindgen makes from the WIT.
//! - `#[activation(describe)]`: the module is left as ordinary Rust and gains
//!   `IR`, `WIT` and `CLIENT_TS`. Everything the IR can say is read; nothing is
//!   wired to run. This is where the caller, grants, roles, children,
//!   progress and a stream in can be written today.

mod write;

use heck::ToKebabCase;
use plexus_ir::{Caller, Child, Deprecated, Field, Held, Ir, Method, Returns, Role, Shape, Ty, TypeDef};
use serde_json::Value;
use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Error, Expr, Fields, FnArg, GenericArgument, ImplItem, Item, ItemMod, Lit, Meta, MetaNameValue, Pat, PathArguments, ReturnType, Token, Type, TypeParamBound};

type Res<T> = Result<T, Error>;

#[proc_macro_attribute]
pub fn activation(args: TokenStream, input: TokenStream) -> TokenStream {
    let module = syn::parse_macro_input!(input as ItemMod);
    expand(args.into(), module).map(|(_, tokens)| tokens).unwrap_or_else(|e| e.to_compile_error()).into()
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
const OURS: [&str; 4] = ["default", "target", "child", "limit"];

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
    let Type::ImplTrait(it) = &**t else { return None };
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

/// What a method gives back, and the error it can end with.
fn returns(ret: &ReturnType) -> Res<(Returns, Option<Ty>)> {
    if let Some(item) = stream_item(ret) {
        return Ok((Returns::Stream(ty(&item)?), None));
    }
    let ReturnType::Type(_, t) = ret else { return Ok((Returns::Nothing, None)) };
    if let Some((ok, e)) = result_of(t)? {
        let ok = match ok {
            Type::Tuple(tu) if tu.elems.is_empty() => Returns::Nothing,
            other => Returns::One(ty(other)?),
        };
        return Ok((ok, Some(ty(e)?)));
    }
    Ok((Returns::One(ty(t)?), None))
}

/// `roles(viewer = ["list"], editor = [viewer, "open"])` of the attribute.
fn roles(args: proc_macro2::TokenStream, activation: &str) -> Res<(bool, Vec<Role>)> {
    let metas = syn::parse::Parser::parse2(Punctuated::<Meta, Token![,]>::parse_terminated, args)?;
    let (mut describe, mut out) = (false, Vec::new());
    for m in metas {
        match &m {
            Meta::Path(p) if p.is_ident("describe") => describe = true,
            Meta::List(l) if l.path.is_ident("roles") => {
                for nv in l.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated)? {
                    let Expr::Array(items) = &nv.value else { return err(&nv.value, "a role is a list: [\"method\", other_role]") };
                    let mut role = Role { name: nv.path.get_ident().map(kebab).unwrap_or_default(), includes: Vec::new(), grants: Vec::new() };
                    for e in &items.elems {
                        match e {
                            Expr::Path(p) => role.includes.push(p.path.get_ident().map(kebab).unwrap_or_default()),
                            other => role.grants.push(format!("{activation}.{}", string(other)?.to_kebab_case())),
                        }
                    }
                    out.push(role);
                }
            }
            other => return err(other, "`describe` or `roles(..)`"),
        }
    }
    Ok((describe, out))
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

/// The module's IR, and the module as a component (or, described, as itself).
fn expand(args: proc_macro2::TokenStream, module: ItemMod) -> Res<(Ir, proc_macro2::TokenStream)> {
    let name = kebab(&module.ident);
    let (describe, roles) = roles(args, &name)?;
    let Some((_, items)) = &module.content else { return err(&module, "#[activation] goes on `mod name { .. }` with its body") };
    let Some(imp) = items.iter().find_map(|i| if let Item::Impl(i) = i { i.trait_.is_none().then_some(i) } else { None }) else {
        return err(&module, "the module needs one `impl` block: the activation's methods");
    };
    let Type::Path(self_ty) = &*imp.self_ty else { return err(&imp.self_ty, "the activation is a named type") };
    let activation = self_ty.path.segments.last().expect("a path has a segment").ident.clone();
    let mut ir = Ir { activation: name.clone(), doc: doc(&module.attrs), types: Vec::new(), methods: Vec::new(), children: Vec::new(), roles };

    // Data: every other struct, enum and alias goes into the IR.
    let (mut kept, mut names) = (Vec::new(), Vec::new());
    for item in items {
        match item {
            // A struct with no fields is no data: an activation of its own, held as a child.
            Item::Struct(s) if s.ident != activation && !matches!(s.fields, Fields::Unit) => {
                let Fields::Named(named) = &s.fields else { return err(s, "a record has named fields") };
                ir.types.push(TypeDef { name: kebab(&s.ident), doc: doc(&s.attrs), shape: Shape::Record(fields_of(named)?) });
                names.push(s.ident.clone());
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
            }
            other => kept.push(other.clone()),
        }
    }

    // Methods and children: every `pub fn` taking `&self` or `&mut self`.
    let iface = format_ident!("{}", module.ident);
    let exports = quote!(self::exports::plexus::#iface::#iface);
    let mut glue = Vec::new();
    for item in &imp.items {
        let ImplItem::Fn(f) = item else { continue };
        if !matches!(f.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let Some(FnArg::Receiver(recv)) = f.sig.inputs.first() else { continue };
        if recv.mutability.is_some() && !describe {
            return err(recv, "an activation's methods take `&self`: two calls can be running at once. Keep what changes in a Cell or a RefCell");
        }
        let method = &f.sig.ident;
        let mut m = Method { name: kebab(method), doc: doc(&f.attrs), params: Vec::new(), returns: Returns::Nothing, fails: None, input: None, progress: None, caller: None, grant: None, target: None, waits: f.sig.asyncness.is_some() || stream_item(&f.sig.output).is_some() };
        let (mut idents, mut tys) = (Vec::new(), Vec::new());
        for arg in f.sig.inputs.iter().skip(1) {
            let FnArg::Typed(t) = arg else { continue };
            let Pat::Ident(p) = &*t.pat else { return err(&t.pat, "a parameter is a plain name") };
            // Three kinds of parameter are not sent as parameters.
            let seg = last_segment(&t.ty);
            let inner = |seg: &syn::PathSegment| type_args(&seg.arguments).first().copied().map(ty).transpose();
            match seg.map(|s| s.ident.to_string()).as_deref() {
                Some("Caller") => m.caller = Some(Caller::Required),
                Some("Option") if seg.and_then(|s| type_args(&s.arguments).first().copied()).and_then(last_segment).is_some_and(|s| s.ident == "Caller") => m.caller = Some(Caller::Optional),
                Some("Input") => m.input = inner(seg.expect("a segment"))?,
                Some("Progress") => m.progress = inner(seg.expect("a segment"))?,
                _ => {
                    let mut field = Field { name: kebab(&p.ident), ty: Some(ty(&t.ty)?), ..Field::default() };
                    extras(&t.attrs, &mut field)?;
                    if attr(&t.attrs, "target").is_some() {
                        m.target = Some(field.name.clone());
                    }
                    m.params.push(field);
                    idents.push(p.ident.clone());
                    tys.push((*t.ty).clone());
                    continue;
                }
            }
            if !describe {
                return err(&t.ty, "the caller, a stream in and progress are not in a component yet: use #[activation(describe)]");
            }
        }
        if let Some(a) = attr(&f.attrs, "child") {
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
            ir.children.push(child);
            continue;
        }
        (m.returns, m.fails) = returns(&f.sig.output)?;
        if !ir.roles.is_empty() || m.target.is_some() {
            m.grant = Some(format!("{name}.{}", m.name));
        }
        if !describe {
            let out = &f.sig.output;
            if let Some((item, iterator)) = stream_of(&f.sig.output) {
                // A stream out: a task of its own walks the method's stream
                // and writes each item into WIT's. The write waits for the
                // reader, and a reader that has gone ends the walk.
                let walk = if iterator { quote!(::futures::stream::iter(made)) } else { quote!(made) };
                glue.push(quote! {
                    async fn #method(#(#idents: #tys),*) -> ::wit_bindgen::StreamReader<#item> {
                        let (mut tx, rx) = self::wit_stream::new();
                        let activation = __ACTIVATION.with(::std::clone::Clone::clone);
                        ::wit_bindgen::spawn_local(async move {
                            let made = activation.#method(#(#idents),*);
                            let mut items = ::std::pin::pin!(#walk);
                            while let ::std::option::Option::Some(item) = ::futures::StreamExt::next(&mut items).await {
                                if !tx.write_all(::std::vec![item]).await.is_empty() {
                                    break;
                                }
                            }
                        });
                        rx
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
        ir.methods.push(m);
    }

    let (wit, json, ts) = (write::wit(&ir), ir.json(), write::ts(&ir));
    let (vis, ident, attrs) = (&module.vis, &module.ident, &module.attrs);
    let consts = quote! {
        /// This activation's IR, as JSON.
        pub const IR: &str = #json;
        /// Its WIT, written from the IR.
        pub const WIT: &str = #wit;
        /// Its typed client and its activation side, in TypeScript, written from the IR.
        pub const CLIENT_TS: &str = #ts;
    };
    if describe {
        let items = plain(items).into_iter().map(|i| i.into_token_stream());
        return Ok((ir, quote! { #(#attrs)* #vis mod #ident { #(#items)* #consts } }));
    }
    if !ir.children.is_empty() {
        return err(&module, "children are not in a component yet: use #[activation(describe)]");
    }
    let world = format!("{}-activation", ir.activation);
    let tokens = quote! {
        #(#attrs)*
        #vis mod #ident {
            ::wit_bindgen::generate!({ inline: #wit, world: #world });
            pub use #exports::{#(#names),*};
            #consts
            #(#kept)*

            ::std::thread_local! {
                static __ACTIVATION: ::std::rc::Rc<#activation> = ::std::rc::Rc::new(<#activation as ::std::default::Default>::default());
            }
            struct __Component;
            impl #exports::Guest for __Component {
                #(#glue)*
            }
            export!(__Component);
        }
    };
    Ok((ir, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "wit" => write::wit(&ir),
            "ts" => write::ts(&ir),
            _ => ir.json(),
        };
        println!("-----8<-----\n{text}-----8<-----");
    }

    /// Every example in `../plexus-examples` writes the IR kept for it in
    /// its `ir/`: a showcase file as `ir/<file>.json`, kiln and glaze as
    /// `ir/kiln.json` and `ir/glaze.json`. `BLESS=1 cargo test goldens`
    /// writes them anew.
    #[test]
    fn goldens() {
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plexus-examples");
        let mut sources: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(examples.join("showcase/src"))
            .expect("plexus-examples beside this checkout")
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_stem().is_some_and(|s| s != "lib"))
            .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
            .collect();
        sources.extend(["kiln", "glaze"].map(|c| (c.to_string(), examples.join(c).join("src/lib.rs"))));
        sources.sort();
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
        let wit = write::wit(&shop().0);
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
                impl Tickets { pub fn status(&self) -> Status { todo!() } }
            }
        };
        let (ir, tokens) = expand(quote!(describe), module).unwrap();
        let json = ir.json_line();
        assert!(json.contains(r#"[{"name": "draft"}, {"name": "in-review", "wire": "In Review"}]"#), "{json}");
        assert!(!tokens.to_string().contains("# [wire"), "{tokens}");
        let ts = write::ts(&ir);
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
        assert!(write::wit(&ir).contains("type ticket-id = string;"), "{}", write::wit(&ir));
    }
}
