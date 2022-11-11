use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, punctuated::Punctuated, token::Comma, Data, DataEnum, DataStruct,
    DeriveInput, Field, Fields,
};

#[proc_macro_derive(Pod)]
pub fn derive_pod(input_token: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_token as DeriveInput);
    expand_derive_pod(input).into()
}

fn expand_derive_pod(input: DeriveInput) -> TokenStream {
    let ident = input.ident;
    let fields = match input.data {
        Data::Struct(DataStruct { fields, .. }) => match fields {
            Fields::Named(fields_named) => fields_named.named,
            Fields::Unnamed(fields_unnamed) => fields_unnamed.unnamed,
            Fields::Unit => Punctuated::new(),
        },
        Data::Enum(DataEnum { variants, .. }) => {
            let mut fields: Punctuated<Field, Comma> = Punctuated::new();
            for var in variants {
                fields.extend(match var.fields {
                    Fields::Named(fields_named) => fields_named.named,
                    Fields::Unnamed(fields_unnamed) => fields_unnamed.unnamed,
                    Fields::Unit => Punctuated::new(),
                })
            }
            fields
        }
        // Panic on compilation time if one tries to derive pod for enum or union.
        // It may not be a good idea, but works now.
        _ => panic!("derive pod only works for struct and enum now."),
    };

    // deal with generics
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    let pod_where_predicates = fields
        .into_iter()
        .map(|field| {
            let field_ty = field.ty;
            quote! {
                #field_ty: ::kxos_frame::Pod
            }
        })
        .collect::<Vec<_>>();

    // if where_clause is none, we should add a `where` word manually.
    if where_clause.is_none() {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::kxos_frame::Pod #type_generics for #ident where #(#pod_where_predicates),* {}
        }
    } else {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::kxos_frame::Pod #type_generics for #ident #where_clause, #(#pod_where_predicates),* {}
        }
    }
}
