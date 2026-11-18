// SPDX-License-Identifier: MPL-2.0

use proc_macro2::{Ident, Span, TokenStream};
use quote::{TokenStreamExt, format_ident, quote};
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Expr, Fields, Generics, Type, parse_macro_input,
};

const ALLOWED_REPRS: &[&str] = &[
    "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "usize", "isize",
];
const VALUE_NAME: &str = "value";
const REPR_PATH: &str = "repr";

#[proc_macro_derive(TryFromInt, attributes(try_from_int))]
pub fn derive_try_from_num(input_token: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input_token as DeriveInput);
    expand_derive_try_from_num(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_derive_try_from_num(input: DeriveInput) -> syn::Result<TokenStream> {
    let attrs = input.attrs;
    let ident = input.ident;
    let generics = input.generics;
    if let Data::Enum(data_enum) = input.data {
        impl_try_from(data_enum, attrs, generics, ident)
    } else {
        Err(syn::Error::new_spanned(
            ident,
            "TryFromInt requires an enum",
        ))
    }
}

fn impl_try_from(
    data_enum: DataEnum,
    attrs: Vec<Attribute>,
    generics: Generics,
    ident: Ident,
) -> syn::Result<TokenStream> {
    let valid_repr = if let Some(valid_repr) = has_valid_repr(&attrs) {
        format_ident!("{}", valid_repr)
    } else {
        return Err(syn::Error::new_spanned(
            &ident,
            "TryFromInt requires a supported integer repr",
        ));
    };

    for variant in &data_enum.variants {
        for attr in &variant.attrs {
            if attr.path().is_ident("try_from_int") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "try_from_int must be applied to the enum",
                ));
            }
        }
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                &variant.fields,
                "TryFromInt requires an enum with only unit variants",
            ));
        }
    }

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let param = Ident::new(VALUE_NAME, Span::mixed_site());
    let (error_type, error_value) = match parse_error_config(&attrs, &param)? {
        Some((error_type, error_value)) => (quote!(#error_type), error_value),
        None => (
            quote!(::int_to_c_enum::TryFromIntError),
            quote!(::int_to_c_enum::TryFromIntError),
        ),
    };
    let fn_body = fn_body_tokens(VALUE_NAME, &data_enum, &ident, &valid_repr, error_value);
    Ok(quote!(
        #[automatically_derived]
        impl #impl_generics ::core::convert::TryFrom<#valid_repr> #type_generics for #ident #where_clause  {
            type Error = #error_type;
            fn try_from(#param: #valid_repr) -> ::core::result::Result<Self, Self::Error> {
                #fn_body
            }
        }
    ))
}

fn parse_error_config(
    attrs: &[Attribute],
    param: &Ident,
) -> syn::Result<Option<(Type, TokenStream)>> {
    let mut error_type = None;
    let mut error_fn = None;
    let mut error_value = None;
    let mut config_attr = None;
    for attr in attrs {
        if !attr.path().is_ident("try_from_int") {
            continue;
        }
        config_attr = Some(attr);
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("error_type") {
                if error_type.is_some() {
                    return Err(meta.error("duplicate error_type"));
                }
                error_type = Some(meta.value()?.parse::<Type>()?);
            } else if meta.path.is_ident("error_fn") {
                if error_fn.is_some() {
                    return Err(meta.error("duplicate error_fn"));
                }
                error_fn = Some(meta.value()?.parse::<Expr>()?);
            } else if meta.path.is_ident("error_value") {
                if error_value.is_some() {
                    return Err(meta.error("duplicate error_value"));
                }
                error_value = Some(meta.value()?.parse::<Expr>()?);
            } else {
                return Err(meta.error("expected error_type, error_fn, or error_value"));
            }
            Ok(())
        })?;
    }
    let Some(attr) = config_attr else {
        return Ok(None);
    };
    match (error_type, error_fn, error_value) {
        (_, Some(_), Some(_)) => Err(syn::Error::new_spanned(
            attr,
            "error_fn and error_value are mutually exclusive",
        )),
        (Some(error_type), Some(error_fn), None) => {
            Ok(Some((error_type, quote!((#error_fn)(#param)))))
        }
        (Some(error_type), None, Some(error_value)) => Ok(Some((error_type, quote!(#error_value)))),
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected error_type and exactly one of error_fn or error_value",
        )),
    }
}

fn fn_body_tokens(
    value_name: &str,
    data_enum: &DataEnum,
    ident: &Ident,
    repr: &Ident,
    error_value: TokenStream,
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
            #discriminant_ident => return ::core::result::Result::Ok(#ident::#variant_ident),
        ));
    }
    let param = Ident::new(value_name, Span::mixed_site());
    quote!(
        // This block limits the scope of the generated discriminant constants.
        // Otherwise, they could shadow names in the caller's `error_value` or
        // `error_fn` expression and silently change the returned error.
        {
            #discriminant_consts
            match #param {
                #match_arms
                _ => {}
            }
        }
        ::core::result::Result::Err(#error_value)
    )
}

fn has_valid_repr(attrs: &[Attribute]) -> Option<&'static str> {
    for attr in attrs {
        if let Some(repr) = parse_repr(attr) {
            return Some(repr);
        }
    }
    None
}

fn parse_repr(attr: &Attribute) -> Option<&'static str> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_error_config() {
        let cases = [
            (
                quote!(#[try_from_int(error_type = CustomError)]),
                "expected error_type and exactly one of error_fn or error_value",
            ),
            (
                quote!(#[try_from_int(error_fn = CustomError::new)]),
                "expected error_type and exactly one of error_fn or error_value",
            ),
            (
                quote!(#[try_from_int()]),
                "expected error_type and exactly one of error_fn or error_value",
            ),
            (
                quote!(#[try_from_int(error_value = CustomError)]),
                "expected error_type and exactly one of error_fn or error_value",
            ),
            (
                quote!(#[try_from_int(unknown = CustomError)]),
                "expected error_type, error_fn, or error_value",
            ),
            (
                quote!(#[try_from_int(error_value = CustomError, error_value = CustomError)]),
                "duplicate error_value",
            ),
            (
                quote!(#[try_from_int(
                    error_type = CustomError,
                    error_fn = CustomError::new,
                    error_value = CustomError,
                )]),
                "error_fn and error_value are mutually exclusive",
            ),
            (
                quote!(
                    #[try_from_int(error_type = CustomError, error_value = CustomError)]
                    #[try_from_int(error_fn = CustomError::new)]
                ),
                "error_fn and error_value are mutually exclusive",
            ),
            (
                quote!(#[try_from_int(error_type = CustomError, error_type = CustomError)]),
                "duplicate error_type",
            ),
            (
                quote!(
                    #[try_from_int(error_fn = CustomError::new)]
                    #[try_from_int(error_fn = CustomError::new)]
                ),
                "duplicate error_fn",
            ),
        ];

        for (attrs, expected) in cases {
            let input = syn::parse_quote!(
                #attrs
                #[repr(u8)]
                enum Value { Valid }
            );
            let error = expand_derive_try_from_num(input).unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn error_config_on_variant() {
        let input = syn::parse_quote!(
            #[repr(u8)]
            enum Value {
                #[try_from_int(error_type = CustomError, error_fn = CustomError::new)]
                Valid,
            }
        );
        let error = expand_derive_try_from_num(input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "try_from_int must be applied to the enum"
        );
    }
}
