// SPDX-License-Identifier: MPL-2.0

//! # padding-struct
//!
//! A procedural macro for automatically adding explicit padding fields to `#[repr(C)]` structs.
//!
//! This crate provides the `#[padding_struct]` attribute macro that transforms a struct definition
//! by automatically inserting padding fields between and after each field to match the memory layout
//! of the C representation.
//!
//! ## Why Use This?
//!
//! When working with `#[repr(C)]` structs, the Rust compiler automatically adds padding to ensure
//! proper alignment. However, sometimes you need explicit control over these padding bytes,
//! for example, ensure padding bytes are initialized when working with `bytemuck` or `zerocopy`.
//!

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Fields, Ident, Token, parse_macro_input,
    punctuated::Punctuated, spanned::Spanned,
};

/// Checks if the struct has a `#[repr(C)]` attribute (possibly with other repr options)
fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("repr") {
            // Parse the attribute using a custom parser
            let result = attr.parse_args_with(Punctuated::<syn::Meta, Token![,]>::parse_terminated);

            if let Ok(list) = result {
                return list
                    .iter()
                    .any(|meta| matches!(meta, syn::Meta::Path(path) if path.is_ident("C")));
            }

            false
        } else {
            false
        }
    })
}

/// Procedural macro to automatically add padding fields to a `#[repr(C)]` struct
///
/// This macro generates two structs:
/// 1. A reference struct (prefixed with `__`) containing the original fields without padding
/// 2. A padded struct (using the original name) with `_padN` padding fields after each original field
///
/// # Padding Calculation Rules
///
/// - For non-last fields: padding size = next field's offset - current field's offset - current field's size
/// - For the last field: padding size = struct total size (or specified size) - current field's offset - current field's size
///
/// # Requirements
///
/// - The struct must have a `#[repr(C)]` attribute
/// - The struct must have named fields
///
/// # Examples
///
/// ```rust
/// use padding_struct::padding_struct;
///
/// #[repr(C)]
/// #[padding_struct]
/// struct MyStruct {
///     a: u8,
///     b: u32,
/// }
/// ```
///
/// This generates code equivalent to:
///
/// ```rust
/// #[repr(C)]
/// struct __MyStruct__ {
///     a: u8,
///     b: u32,
/// }
///
/// #[repr(C)]
/// struct MyStruct {
///     a: u8,
///     pub __pad1: [u8; { core::mem::offset_of!(__MyStruct__, b) - core::mem::offset_of!(__MyStruct__, a) - core::mem::size_of::<u8>() }],
///     b: u32,
///     pub __pad2: [u8; { core::mem::size_of::<__MyStruct__>() - core::mem::offset_of!(__MyStruct__, b) - core::mem::size_of::<u32>() }],
/// }
/// ```
///
#[proc_macro_attribute]
pub fn padding_struct(args: TokenStream, input: TokenStream) -> TokenStream {
    // Reject any provided arguments to the attribute
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[padding_struct]` does not accept any arguments",
        )
        .to_compile_error()
        .into();
    }
    let input = parse_macro_input!(input as DeriveInput);

    // Ensure it's a struct
    let fields = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => &fields.named,
        _ => panic!("`#[padding_struct]` only supports named-field structs"),
    };

    // Ensure #[repr(C)] is present
    if !has_repr_c(&input.attrs) {
        panic!("`#[padding_struct]` requires the struct to be `#[repr(C)]`");
    }

    let name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let ref_name = Ident::new(&format!("__{}__", name), name.span());

    // Filter attributes for the padded struct (keep all except #[padding_struct])
    let padded_attrs: Vec<_> = input
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("padding_struct"))
        .collect();

    // Generate reference struct (same fields, no padding, only #[repr(C)])
    let ref_fields: Vec<_> = fields.iter().collect();
    let ref_struct = quote! {
        #[repr(C)]
        #[allow(missing_docs)]
        #[doc(hidden)]
        struct #ref_name #impl_generics #where_clause {
            #(#ref_fields),*
        }
    };

    // Generate padded struct with inline const expressions
    let mut padded_fields = Vec::new();
    let field_vec: Vec<_> = fields.iter().collect();

    for (i, field) in field_vec.iter().enumerate() {
        let field_name = &field.ident;
        let field_ty = &field.ty;
        let field_attrs = &field.attrs;
        let field_vis = &field.vis;

        // Add original field with its attributes and comments
        padded_fields.push(quote! {
            #(#field_attrs)*
            #field_vis #field_name: #field_ty
        });

        // Generate padding field with inline const expression
        let pad_num = i + 1;
        let pad_ident = Ident::new(&format!("__pad{}", pad_num), field.span());

        let pad_size_expr = if i == field_vec.len() - 1 {
            // Last field: padding to end of struct (no external size supported)
            quote! {
                core::mem::size_of::<#ref_name #ty_generics>()
                    - core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - core::mem::size_of::<#field_ty>()
            }
        } else {
            // Middle field: padding to next field
            let next_field = field_vec[i + 1];
            let next_field_name = &next_field.ident;
            quote! {
                core::mem::offset_of!(#ref_name #ty_generics, #next_field_name)
                    - core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - core::mem::size_of::<#field_ty>()
            }
        };

        // Add padding field with inline const block
        padded_fields.push(quote! {
            #[allow(missing_docs)]
            pub #pad_ident: [u8; { #pad_size_expr }]
        });
    }

    let padded_struct = quote! {
        #(#padded_attrs)*
        #vis struct #name #impl_generics #where_clause {
            #(#padded_fields),*
        }
    };

    let expanded = quote! {
        #ref_struct

        #padded_struct
    };

    TokenStream::from(expanded)
}
