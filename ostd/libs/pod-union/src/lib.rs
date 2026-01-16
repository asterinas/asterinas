// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

/// Derive macro to automatically implement `bytemuck::Pod` and `bytemuck::Zeroable` for unions
///
/// **FIXME: The current implementation is unsound, since it temporarily disables field size check.**
///
/// This macro checks that all fields in the union have the same size, then generates
/// implementations of `Pod` and `Zeroable` traits from the `bytemuck` crate.
///
/// # Requirements
///
/// - All fields must implement `bytemuck::Pod` (which also requires `Zeroable`)
/// - All fields must have the same size
///
/// # Examples
///
/// Success case - all fields are 128 bits:
///
/// ```rust,ignore
/// use padding_struct::PodUnion;
///
/// #[derive(PodUnion)]
/// union B {
///     a: [u8; 128],
///     b: [u32; 32],
///     c: u128,
/// }
/// ```
///
/// Failure case - fields have different sizes:
///
/// ```rust,compile_fail
/// use padding_struct::PodUnion;
///
/// #[derive(PodUnion)]
/// union A {
///     a: u32,    // 4 bytes
///     b: u64,    // 8 bytes
///     c: u128,   // 16 bytes
/// }
/// ```
#[proc_macro_derive(PodUnion)]
pub fn derive_pod_union(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Ensure it's a union
    let fields = match &input.data {
        Data::Union(data_union) => &data_union.fields,
        _ => {
            return syn::Error::new_spanned(name, "`#[derive(PodUnion)]` only supports unions")
                .to_compile_error()
                .into();
        }
    };

    // Get all field types
    let field_types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();

    if field_types.is_empty() {
        return syn::Error::new_spanned(name, "`#[derive(PodUnion)]` requires at least one field")
            .to_compile_error()
            .into();
    }

    // let first_field_ty = field_types[0];

    // Generate size equality checks - const array size mismatch will produce clear errors
    // let size_checks: Vec<_> = field_types
    //     .iter()
    //     .skip(1)
    //     .map(|field_ty| {
    //         quote! {
    //             const _: [(); core::mem::size_of::<#first_field_ty>()] =
    //                 [(); core::mem::size_of::<#field_ty>()];
    //         }
    //     })
    //     .collect();

    // Generate where clause that requires all fields to be Pod
    let pod_bounds: Vec<_> = field_types
        .iter()
        .map(|field_ty| {
            quote! { #field_ty: bytemuck::Pod }
        })
        .collect();

    // Generate the implementations
    let expanded = quote! {
        // const _: () = {
        //     #(#size_checks)*
        // };

        unsafe impl #impl_generics bytemuck::Zeroable for #name #ty_generics #where_clause {}

        unsafe impl #impl_generics bytemuck::Pod for #name #ty_generics
        where
            #(#pod_bounds),*
        {}
    };

    TokenStream::from(expanded)
}
