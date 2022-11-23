//! This crate is used to provide a procedural macro to derive Pod trait defined in framework/pod.
//! When use this crate, framework/pod should also be added as a dependency.
//! This macro should only be used outside
//! When derive Pod trait, we will do a check whether the derive is safe since Pod trait is an unsafe trait.
//! For struct, we will check that the struct has valid repr (e.g,. repr(C), repr(u8)), and each field is Pod type.
//! For union and enum, we only check the valid repr.

use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{
    parse_macro_input, Attribute, Data, DataEnum, DataStruct, DataUnion, DeriveInput, Fields,
    Generics,
};

#[proc_macro_derive(Pod)]
pub fn derive_pod(input_token: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_token as DeriveInput);
    expand_derive_pod(input).into()
}

const ALLOWED_REPRS: [&'static str; 11] = [
    "C", "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "usize", "isize",
];

fn expand_derive_pod(input: DeriveInput) -> TokenStream {
    let attrs = input.attrs;
    let ident = input.ident;
    let generics = input.generics;
    match input.data {
        Data::Struct(data_struct) => impl_pod_for_struct(data_struct, generics, ident, attrs),
        Data::Union(data_union) => impl_pod_for_union(data_union, generics, ident, attrs),
        Data::Enum(data_enum) => impl_pod_for_enum(data_enum, attrs, generics, ident),
    }
}

fn impl_pod_for_struct(
    data_struct: DataStruct,
    generics: Generics,
    ident: Ident,
    attrs: Vec<Attribute>,
) -> TokenStream {
    if !has_valid_repr(attrs) {
        panic!("{} has invalid repr to implement Pod", ident.to_string());
    }
    let DataStruct { fields, .. } = data_struct;
    let fields = match fields {
        Fields::Named(fields_named) => fields_named.named,
        Fields::Unnamed(fields_unnamed) => fields_unnamed.unnamed,
        Fields::Unit => panic!("derive pod does not work for struct with unit field"),
    };

    // deal with generics
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let pod_where_predicates = fields
        .into_iter()
        .map(|field| {
            let field_ty = field.ty;
            quote! {
                #field_ty: ::pod::Pod
            }
        })
        .collect::<Vec<_>>();

    // if where_clause is none, we should add a `where` word manually.
    if where_clause.is_none() {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::pod::Pod #type_generics for #ident where #(#pod_where_predicates),* {}
        }
    } else {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::pod::Pod #type_generics for #ident #where_clause, #(#pod_where_predicates),* {}
        }
    }
}

fn impl_pod_for_union(
    data_union: DataUnion,
    generics: Generics,
    ident: Ident,
    attrs: Vec<Attribute>,
) -> TokenStream {
    if !has_valid_repr(attrs) {
        panic!("{} has invalid repr to implement Pod", ident.to_string());
    }
    let fields = data_union.fields.named;
    // deal with generics
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let pod_where_predicates = fields
        .into_iter()
        .map(|field| {
            let field_ty = field.ty;
            quote! {
                #field_ty: ::pod::Pod
            }
        })
        .collect::<Vec<_>>();

    // if where_clause is none, we should add a `where` word manually.
    if where_clause.is_none() {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::pod::Pod #type_generics for #ident where #(#pod_where_predicates),* {}
        }
    } else {
        quote! {
            #[automatically_derived]
            unsafe impl #impl_generics ::pod::Pod #type_generics for #ident #where_clause, #(#pod_where_predicates),* {}
        }
    }
}

fn impl_pod_for_enum(
    data_enum: DataEnum,
    attrs: Vec<Attribute>,
    generics: Generics,
    ident: Ident,
) -> TokenStream {
    if !has_valid_repr(attrs) {
        panic!(
            "{} does not have invalid repr to implement Pod.",
            ident.to_string()
        );
    }

    // check variant
    for variant in data_enum.variants {
        if None == variant.discriminant {
            panic!("Enum can only have fields like Variant=1");
        }
    }

    // deal with generics
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        unsafe impl #impl_generics ::pod::Pod #type_generics for #ident #where_clause {}
    }
}

fn has_valid_repr(attrs: Vec<Attribute>) -> bool {
    for attr in attrs {
        if let Some(ident) = attr.path.get_ident() {
            if "repr" == ident.to_string().as_str() {
                let repr = attr.tokens.to_string();
                let repr = repr.replace("(", "").replace(")", "");
                let reprs = repr
                    .split(",")
                    .map(|one_repr| one_repr.trim())
                    .collect::<Vec<_>>();
                if let Some(_) = ALLOWED_REPRS.iter().position(|allowed_repr| {
                    reprs
                        .iter()
                        .position(|one_repr| one_repr == allowed_repr)
                        .is_some()
                }) {
                    return true;
                }
            }
        }
    }
    false
}
