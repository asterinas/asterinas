// SPDX-License-Identifier: MPL-2.0

#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Fields, Ident, Token, parse_macro_input,
    punctuated::Punctuated, spanned::Spanned,
};

/// Checks if the struct has a `#[repr(C)]` attribute (possibly with other `repr` options)
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

/// Extracts all `#[repr(...)]` attributes from the given attributes
fn extract_repr_attrs(attrs: &[Attribute]) -> Vec<&Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("repr"))
        .collect()
}

/// Procedural macro to automatically add padding fields to a `#[repr(C)]` struct.
///
/// This macro generates two structs:
/// 1. A reference struct (prefixed and suffixed with `__`) containing the original fields without padding.
/// 2. A padded struct (using the original name) with `__padN` padding fields after each original field.
///
/// # Padding Calculation Rules
///
/// - For non-last fields: padding size = next field's offset - current field's offset - current field's size.
/// - For the last field: padding size = struct total size - current field's offset - current field's size.
///
/// # Requirements
///
/// - The struct must have a `#[repr(C)]` attribute.
/// - The struct must have named fields.
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
/// use core::mem::offset_of;
///
/// #[repr(C)]
/// struct __MyStruct__ {
///     a: u8,
///     b: u32,
/// }
///
/// #[repr(C)]
/// struct MyStruct {
///     a: u8,
///     pub __pad1:
///         [u8; const { offset_of!(__MyStruct__, b) - offset_of!(__MyStruct__, a) - size_of::<u8>() }],
///     b: u32,
///     pub __pad2:
///         [u8; const { size_of::<__MyStruct__>() - offset_of!(__MyStruct__, b) - size_of::<u32>() }],
/// }
/// ```
#[proc_macro_attribute]
pub fn padding_struct(args: TokenStream, input: TokenStream) -> TokenStream {
    // Reject any provided arguments to the attribute
    if !args.is_empty() {
        panic!("`#[padding_struct]` does not accept any arguments");
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
        panic!("`#[padding_struct]` requires `#[repr(C)]` or `#[repr(C, ...)]` on struct");
    }

    let name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let ref_name = Ident::new(&format!("__{}__", name), name.span());

    // Extract all repr attributes for the reference struct
    let repr_attrs = extract_repr_attrs(&input.attrs);

    // Generate reference struct (same fields, no padding, with all repr attributes)
    let ref_fields: Vec<_> = fields.iter().collect();
    let ref_struct = quote! {
        #(#repr_attrs)*
        #[allow(missing_docs)]
        #[doc(hidden)]
        struct #ref_name #impl_generics #where_clause {
            #(#ref_fields),*
        }
    };

    // Filter attributes for the padded struct (keep all except #[padding_struct])
    let padded_attrs: Vec<_> = input
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("padding_struct"))
        .collect();

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
            // Last field: padding to end of struct
            quote! {
                ::core::mem::size_of::<#ref_name #ty_generics>()
                    - ::core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - ::core::mem::size_of::<#field_ty>()
            }
        } else {
            // Middle field: padding to next field
            let next_field = field_vec[i + 1];
            let next_field_name = &next_field.ident;
            quote! {
                ::core::mem::offset_of!(#ref_name #ty_generics, #next_field_name)
                    - ::core::mem::offset_of!(#ref_name #ty_generics, #field_name)
                    - ::core::mem::size_of::<#field_ty>()
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

    // Generate compile-time assertions to ensure size and alignment match
    let size_align_check = quote! {
        const _: () = {
            // Assert that sizes are equal
            const _: [(); ::core::mem::size_of::<#ref_name #ty_generics>()] =
                [(); ::core::mem::size_of::<#name #ty_generics>()];

            // Assert that alignments are equal
            const _: [(); ::core::mem::align_of::<#ref_name #ty_generics>()] =
                [(); ::core::mem::align_of::<#name #ty_generics>()];
        };
    };

    let expanded = quote! {
        #ref_struct

        #padded_struct

        #size_align_check
    };

    TokenStream::from(expanded)
}
