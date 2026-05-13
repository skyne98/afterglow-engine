use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{DeriveInput, Generics, Item, parse_macro_input, parse_quote};

#[proc_macro_derive(Replicate)]
pub fn derive_replicate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    impl_replicate(&input)
}

#[proc_macro_attribute]
pub fn replicate(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let replicate = match &item {
        Item::Struct(input) => impl_replicate_for_ident(&input.ident, &input.generics),
        Item::Enum(input) => impl_replicate_for_ident(&input.ident, &input.generics),
        _ => {
            return quote! {
                compile_error!("#[replicate] can only be used on structs or enums");
                #item
            }
            .into();
        }
    };

    quote! {
        #item
        #replicate
    }
    .into()
}

fn impl_replicate(input: &DeriveInput) -> TokenStream {
    impl_replicate_for_ident(&input.ident, &input.generics).into()
}

fn impl_replicate_for_ident(ident: &syn::Ident, generics: &Generics) -> proc_macro2::TokenStream {
    let mut generics = generics.clone();
    let (_, type_generics, _) = generics.split_for_impl();
    let self_type: syn::Type = parse_quote!(#ident #type_generics);
    let engine = afterglow_engine_path();
    generics
        .make_where_clause()
        .predicates
        .push(parse_quote!(#self_type: ::core::marker::Send + ::core::marker::Sync + 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics #engine::network::replication::Replicate
            for #ident #type_generics #where_clause
        {
            const REPLICATION_NAME: &'static str = concat!(module_path!(), "::", stringify!(#ident));
        }
    }
}

fn afterglow_engine_path() -> proc_macro2::TokenStream {
    match crate_name("afterglow-engine") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{name}");
            quote!(::#ident)
        }
        Err(_) => quote!(::afterglow_engine),
    }
}
