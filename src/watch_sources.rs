//! Expand change-source method calls in the activation's own Rust scope.
use super::{Res, err};
use quote::{format_ident, quote};
use syn::{Expr, FnArg, Ident, ImplItemFn};

pub(super) fn call(expr: &Expr) -> Res<(String, proc_macro2::TokenStream)> {
    let (inner, fallible) = match expr {
        Expr::Try(t) => (&*t.expr, true),
        other => (other, false),
    };
    let Expr::MethodCall(call) = inner else {
        return err(
            expr,
            "expected a dependency name or a change-source method call",
        );
    };
    fn receiver(expr: &Expr) -> Res<(Vec<String>, proc_macro2::TokenStream)> {
        match expr {
            Expr::Path(p) if p.path.is_ident("self") => Ok((Vec::new(), quote!(self))),
            Expr::Path(p) if p.path.get_ident().is_some() => {
                let field = p.path.get_ident().unwrap();
                Ok((vec![field.to_string()], quote!(self.#field)))
            }
            Expr::Field(f) => {
                let (mut names, receiver) = receiver(&f.base)?;
                let member = &f.member;
                names.push(quote!(#member).to_string());
                Ok((names, quote!(#receiver.#member)))
            }
            _ => err(
                expr,
                "a change source is a method of the activation or one of its fields",
            ),
        }
    }
    let (mut path, receiver) = receiver(&call.receiver)?;
    let (method, args, types) = (&call.method, &call.args, &call.turbofish);
    path.push(method.to_string());
    let propagate = fallible.then(|| quote!(?));
    Ok((
        path.join("."),
        quote!(#receiver.#method #types (#args) #propagate),
    ))
}

pub(super) fn helper_name(method: &Ident) -> Ident {
    format_ident!("__plexus_subscribe_{}", method)
}

pub(super) fn helper(
    f: &ImplItemFn,
    sources: &[proc_macro2::TokenStream],
) -> proc_macro2::TokenStream {
    let name = helper_name(&f.sig.ident);
    let mut inputs = f.sig.inputs.clone();
    for input in &mut inputs {
        if let FnArg::Typed(arg) = input {
            arg.attrs
                .retain(|a| !super::OURS.iter().any(|key| a.path().is_ident(key)));
        }
    }
    quote! {
        #[allow(unused_variables)]
        fn #name(#inputs) -> ::std::result::Result<::plexus::Invalidations, ::plexus::Failure> {
            let sources: ::std::vec::Vec<::plexus::Invalidations> = ::std::vec![#(::std::boxed::Box::pin(#sources)),*];
            ::std::result::Result::Ok(::plexus::merge_invalidations(sources))
        }
    }
}
