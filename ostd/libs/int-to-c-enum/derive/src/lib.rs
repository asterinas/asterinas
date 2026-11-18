// SPDX-License-Identifier: MPL-2.0

use proc_macro2::{Ident, TokenStream};
use quote::{TokenStreamExt, format_ident, quote};
use syn::{Attribute, Data, DataEnum, DeriveInput, Fields, Generics, parse_macro_input};

const ALLOWED_REPRS: &[&str] = &[
    "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "usize", "isize",
];
const VALUE_NAME: &str = "value";
const REPR_PATH: &str = "repr";

#[proc_macro_derive(TryFromInt)]
pub fn derive_try_from_num(input_token: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_token as DeriveInput);
    expand_derive_try_from_num(input).into()
}

fn expand_derive_try_from_num(input: DeriveInput) -> TokenStream {
    let attrs = input.attrs;
    let ident = input.ident;
    let generics = input.generics;
    if let Data::Enum(data_enum) = input.data {
        impl_try_from(data_enum, attrs, generics, ident)
    } else {
        panic!("cannot derive TryFromInt for structs or union.")
    }
}

fn impl_try_from(
    data_enum: DataEnum,
    attrs: Vec<Attribute>,
    generics: Generics,
    ident: Ident,
) -> TokenStream {
    let valid_repr = if let Some(valid_repr) = has_valid_repr(attrs) {
        format_ident!("{}", valid_repr)
    } else {
        panic!(
            "{} does not have invalid repr to implement TryFromInt.",
            ident
        );
    };

    for variant in &data_enum.variants {
        if !matches!(variant.fields, Fields::Unit) {
            panic!("TryFromInt requires an enum with only unit variants");
        }
    }

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let fn_body = fn_body_tokens(VALUE_NAME, &data_enum, &ident, &valid_repr);
    let param = format_ident!("{}", VALUE_NAME);
    quote!(
        #[automatically_derived]
        impl #impl_generics ::core::convert::TryFrom<#valid_repr> #type_generics for #ident #where_clause  {
            type Error = ::int_to_c_enum::TryFromIntError;
            fn try_from(#param: #valid_repr) -> ::core::result::Result<Self, Self::Error> {
                #fn_body
            }
        }
    )
}

fn fn_body_tokens(
    value_name: &str,
    data_enum: &DataEnum,
    ident: &Ident,
    repr: &Ident,
) -> TokenStream {
    let mut discriminant_consts = quote!();
    let mut match_arms = quote!();
    for (index, variant) in data_enum.variants.iter().enumerate() {
        let variant_ident = &variant.ident;
        let discriminant_ident = format_ident!("__TRY_FROM_INT_DISCRIMINANT_{}", index);
        discriminant_consts.append_all(quote!(
            const #discriminant_ident: #repr = #ident::#variant_ident as #repr;
        ));
        match_arms.append_all(quote!(
            #discriminant_ident => ::core::result::Result::Ok(#ident::#variant_ident),
        ));
    }
    let param = format_ident!("{}", value_name);
    quote!(
        #discriminant_consts
        match #param {
            #match_arms
            _ => ::core::result::Result::Err(::int_to_c_enum::TryFromIntError),
        }
    )
}

fn has_valid_repr(attrs: Vec<Attribute>) -> Option<&'static str> {
    for attr in attrs {
        if let Some(repr) = parse_repr(attr) {
            return Some(repr);
        }
    }
    None
}

fn parse_repr(attr: Attribute) -> Option<&'static str> {
    if !attr.path().is_ident(REPR_PATH) {
        return None;
    }
    let mut repr = None;
    attr.parse_nested_meta(|meta| {
        for allowed_repr in ALLOWED_REPRS {
            if meta.path.is_ident(*allowed_repr) {
                repr = Some(*allowed_repr);
                break;
            }
        }
        Ok(())
    })
    .ok()?;
    repr
}
