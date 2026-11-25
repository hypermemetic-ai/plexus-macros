//! The Rust client uses the same names, wire arguments and codecs as dispatch.
use super::{MethodArgs, Res, is_bytes, result_of, stream_item};
use plexus_ir::{Ir, Method, Returns};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{ImplItemFn, ReturnType, Type};

pub(super) fn method(ir: &Ir, m: &Method, f: &ImplItemFn, args: &MethodArgs) -> Res<TokenStream> {
    let method = &f.sig.ident;
    let address = format!("{}.{}", ir.activation, m.name);
    let names = &args.wire_names;
    let types = &args.wire_types;
    let fields = m.params.iter().zip(names).map(|(field, name)| {
        let key = &field.name;
        let value = if field.ty.as_ref().is_some_and(|t| is_bytes(ir,t)) {
            quote!(::plexus::__private::B64(#name))
        } else { quote!(#name) };
        quote!(__plexus_params.insert(#key, ::plexus::raw(&#value).map_err(::plexus::Error::Failed)?);)
    });
    let params = quote! {
        let mut __plexus_params = ::std::collections::BTreeMap::<&str, ::plexus::Raw>::new();
        #(#fields)*
    };
    let bytes_input = m.input.as_ref().is_some_and(|t| is_bytes(ir, t));
    let input_arg = args.input_type.as_ref().map(|t| quote! {
        __plexus_input: impl ::plexus::__private::futures::Stream<Item = ::std::result::Result<#t, ::plexus::Failure>> + Send + 'static,
    });
    let upload = args.input_type.as_ref().map(|_| if bytes_input {
        quote!(, ::plexus::__private::futures::TryStreamExt::map_ok(__plexus_input, ::plexus::bytes::Bytes::from))
    } else {
        quote!(, ::plexus::__private::futures::StreamExt::map(__plexus_input, |item| item.and_then(|v| ::plexus::raw(&v))))
    });
    let stream = stream_item(&f.sig.output);
    let op = match (stream.is_some(), m.input.is_some(), bytes_input) {
        (true, true, true) => "stream_with_byte_input",
        (true, true, false) => "stream_with_input",
        (true, false, _) => "stream",
        (false, true, true) => "call_with_byte_input",
        (false, true, false) => "call_with_input",
        (false, false, _) => "call",
    };
    let op = quote::format_ident!("{op}");
    let invoke = quote!(self.0.#op(#address, __plexus_params #upload));
    let bytes_out = match &m.returns {
        Returns::Stream(t) | Returns::One(t) => is_bytes(ir, t),
        _ => false,
    };
    let decode = if bytes_out {
        quote!(|v| ::plexus::__private::peer_decode::<::plexus::__private::B64>(v).map(|v| v.0))
    } else {
        quote!(::plexus::__private::peer_decode)
    };
    if let Some(item) = stream {
        return Ok(quote! {
            pub fn #method(&self, #(#names:#types,)* #input_arg) -> ::std::result::Result<
                ::plexus::__private::futures::stream::BoxStream<'static, ::std::result::Result<#item,::plexus::Error>>, ::plexus::Error> {
                #params
                Ok(::std::boxed::Box::pin(::plexus::__private::futures::StreamExt::map(#invoke?, |item| item.and_then(#decode))))
            }
        });
    }
    let out = match &f.sig.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) => {
            let ok = result_of(ty)?.map_or(&**ty, |(ok, _)| ok);
            if let Type::Reference(r) = ok {
                let t = &r.elem;
                quote!(<#t as ::std::borrow::ToOwned>::Owned)
            } else {
                quote!(#ok)
            }
        }
    };
    Ok(quote! {
        pub async fn #method(&self, #(#names:#types,)* #input_arg) -> ::std::result::Result<#out, ::plexus::Error> {
            #params
            #invoke.await.and_then(#decode)
        }
    })
}
