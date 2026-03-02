// SPDX-License-Identifier: MPL-2.0

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::{
    Attribute, Data, DeriveInput, Ident, Path, Token, Visibility, parse_quote,
    punctuated::Punctuated, spanned::Spanned,
};

const DERIVE_IDENT: &str = "derive";
const REPR_IDENT: &str = "repr";

/// Splits attributes into non-derive attributes and derive paths
fn split_attrs(attrs: Vec<Attribute>) -> (Vec<Attribute>, Vec<Path>) {
    let mut other_attrs = Vec::new();
    let mut derive_paths = Vec::new();

    for attr in attrs {
        if attr.path().is_ident(DERIVE_IDENT) {
            let parsed: Punctuated<Path, Token![,]> = attr
                .parse_args_with(Punctuated::parse_terminated)
                .expect("failed to parse derive attribute");
            derive_paths.extend(parsed.into_iter());
        } else {
            other_attrs.push(attr);
        }
    }

    (other_attrs, derive_paths)
}

/// Checks if the attributes contain `#[repr(C)]`
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

/// Inserts a path into the vector if it's not already present
fn insert_if_absent(paths: &mut Vec<Path>, new_path: Path) {
    let new_repr = new_path.to_token_stream().to_string();
    if !paths
        .iter()
        .any(|path| path.to_token_stream().to_string() == new_repr)
    {
        paths.push(new_path);
    }
}

pub fn expand_pod_union(input: DeriveInput) -> TokenStream2 {
    if !has_repr_c(&input.attrs) {
        panic!("`#[pod_union]` requires `#[repr(C)]` or `#[repr(C, ...)]` on unions");
    }

    let data_union = match input.data {
        Data::Union(ref u) => u,
        _ => panic!("`#[pod_union]` can only be used on unions"),
    };

    let vis: Visibility = input.vis.clone();
    let ident = &input.ident;
    let internal_ident = Ident::new(&format!("__{}__", ident), ident.span());
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Split attributes: keep non-derive attrs, collect derive paths
    let (other_attrs, derive_paths) = split_attrs(input.attrs.clone());

    let mut union_derive_paths = derive_paths.clone();
    let mut struct_derive_paths = derive_paths;

    // Add required zerocopy derives for internal union
    insert_if_absent(&mut union_derive_paths, parse_quote!(::zerocopy::FromBytes));
    insert_if_absent(&mut union_derive_paths, parse_quote!(::zerocopy::Immutable));
    insert_if_absent(
        &mut union_derive_paths,
        parse_quote!(::zerocopy::KnownLayout),
    );

    // Add required zerocopy derives for public struct wrapper
    insert_if_absent(
        &mut struct_derive_paths,
        parse_quote!(::zerocopy::FromBytes),
    );
    insert_if_absent(
        &mut struct_derive_paths,
        parse_quote!(::zerocopy::Immutable),
    );
    insert_if_absent(
        &mut struct_derive_paths,
        parse_quote!(::zerocopy::IntoBytes),
    );
    insert_if_absent(
        &mut struct_derive_paths,
        parse_quote!(::zerocopy::KnownLayout),
    );

    let union_derive_attr: Attribute = parse_quote! {
        #[derive(#(#union_derive_paths),*)]
    };

    let struct_derive_attr: Attribute = parse_quote! {
        #[derive(#(#struct_derive_paths),*)]
    };

    let mut union_attrs = other_attrs.clone();
    union_attrs.push(union_derive_attr);

    let mut struct_attrs: Vec<Attribute> = other_attrs
        .into_iter()
        .filter(|attr| !attr.path().is_ident(REPR_IDENT))
        .collect();
    struct_attrs.push(parse_quote!(#[repr(transparent)]));
    struct_attrs.push(struct_derive_attr);

    let mut internal_union = input.clone();
    internal_union.ident = internal_ident.clone();
    internal_union.vis = Visibility::Inherited;
    internal_union.attrs = union_attrs;

    // Generate accessor methods for each field
    let accessor_methods = data_union.fields.named.iter().map(|field| {
        let field_name = &field.ident;
        let field_ty = &field.ty;

        let ref_method_name = field_name;
        let mut_method_name = syn::Ident::new(
            &format!("{}_mut", field_name.as_ref().unwrap()),
            field_name.span(),
        );

        quote! {
            pub fn #ref_method_name(&self) -> &#field_ty {
                use ::zerocopy::IntoBytes;
                let bytes = self.0.as_bytes();
                let slice = &bytes[..::core::mem::size_of::<#field_ty>()];
                <#field_ty as ::zerocopy::FromBytes>::ref_from_bytes(slice).unwrap()
            }

            pub fn #mut_method_name(&mut self) -> &mut #field_ty {
                use ::zerocopy::IntoBytes;
                let bytes = self.0.as_mut_bytes();
                let slice = &mut bytes[..::core::mem::size_of::<#field_ty>()];
                <#field_ty as ::zerocopy::FromBytes>::mut_from_bytes(slice).unwrap()
            }
        }
    });

    // Generate initializer methods for each field
    let init_methods = data_union.fields.named.iter().map(|field| {
        let field_name = field.ident.as_ref().expect("field name");
        let field_ty = &field.ty;
        let new_method_name = syn::Ident::new(&format!("new_{}", field_name), field_name.span());
        let mut_method_name = syn::Ident::new(&format!("{}_mut", field_name), field_name.span());

        quote! {
            #[allow(non_snake_case)]
            pub fn #new_method_name(value: #field_ty) -> Self {
                use ::zerocopy::FromZeros;
                let mut slf = Self::new_zeroed();
                *slf.#mut_method_name() = value;
                slf
            }
        }
    });

    // Generate module name to avoid symbol conflicts
    let module_ident = syn::Ident::new(
        &format!(
            "__private_module_generated_by_ostd_pod_{}",
            ident.to_string().to_lowercase()
        ),
        proc_macro2::Span::call_site(),
    );

    // Add Copy constraint compile-time assertion
    let copy_assert = quote! {
        const _: () = {
            fn assert_copy<T: ::core::marker::Copy>() {}
            fn assert_union_copy #impl_generics() #where_clause {
                assert_copy::<#ident #ty_generics>();
            }
        };
    };

    // Generate Pod constraint assertions for all fields
    let field_pod_asserts = data_union.fields.named.iter().map(|field| {
        let ty = &field.ty;
        quote! {
            assert_pod::<#ty>();
        }
    });
    let pod_assert = quote! {
        const _: () = {
            fn assert_pod<T: ::ostd_pod::Pod>() {}
            fn assert_union_fields #impl_generics() #where_clause {
                #(#field_pod_asserts)*
            }
        };
    };

    // Generate the public struct
    let size_expr = quote!({ ::core::mem::size_of::<#internal_ident #ty_generics>() });
    let align_expr = quote! ({::core::mem::align_of::<#internal_ident #ty_generics>()});
    let size_align_assert = quote! {
        const _: () = {
            let size = #size_expr;
            let align = #align_expr;
            assert!(size % align == 0, "size must be a multiple of align");
        };
    };
    let internal_array = quote! {
        <::ostd_pod::array_helper::ArrayFactory<
                { (#align_expr) },
                { (#size_expr) / (#align_expr) }
            > as ::ostd_pod::array_helper::ArrayManufacture>::Array
    };
    let public_struct = quote! {
        #(#struct_attrs)*
        pub struct #ident #impl_generics(#internal_array) #where_clause;
    };

    quote! {
        mod #module_ident {
            use super::*;

            #internal_union

            #public_struct

            impl #impl_generics #ident #ty_generics #where_clause {
                #(#accessor_methods)*
                #(#init_methods)*
            }

            #pod_assert
            #copy_assert
            #size_align_assert
        }

        #vis use #module_ident::#ident;
    }
}
