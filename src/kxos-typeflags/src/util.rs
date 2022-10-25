use proc_macro2::TokenStream;
use quote::{quote, TokenStreamExt};

use crate::{
    flag_set::{generate_flag_sets, FlagSet},
    type_flag::TypeFlagDef,
};

pub fn expand_type_flag(type_flags_def: &TypeFlagDef) -> TokenStream {
    let mut all_tokens = TokenStream::new();
    let import_util_tokens = import_util();
    all_tokens.append_all(import_util_tokens);

    let trait_and_items_def_tokens = trait_and_items_def(type_flags_def);
    all_tokens.append_all(trait_and_items_def_tokens);

    let impl_same_as_tokens = impl_same_as(type_flags_def);
    all_tokens.append_all(impl_same_as_tokens);

    let flag_sets = generate_flag_sets(&type_flags_def);
    flag_sets.iter().for_each(|flag_set| {
        let impl_main_trait_tokens = flag_set.impl_main_trait_tokens(type_flags_def);
        all_tokens.append_all(impl_main_trait_tokens);
    });

    let export_declarive_macro_tokens = export_declarive_macro(type_flags_def, &flag_sets);
    all_tokens.append_all(export_declarive_macro_tokens);

    all_tokens
}

/// import crate kxos_typeflags_util
pub fn import_util() -> TokenStream {
    quote!(
        #[macro_use]
        use ::kxos_typeflags_util::*;
    )
}

/// define the main trait and all items
pub fn trait_and_items_def(type_flags_def: &TypeFlagDef) -> TokenStream {
    let mut tokens = TokenStream::new();
    tokens.append_all(type_flags_def.trait_def_tokens());
    for item_def in type_flags_def.items_def_tokens() {
        tokens.append_all(item_def);
    }
    tokens
}

/// impl SameAs trait for each struct
pub fn impl_same_as(type_flags_def: &TypeFlagDef) -> TokenStream {
    let mut all_tokens = TokenStream::new();
    let items_num = type_flags_def.item_num();

    for i in 0..items_num {
        let item1 = type_flags_def.get_item(i).unwrap();
        for j in 0..items_num {
            if i == j {
                // We don't need to impl SameAs for the same type
                continue;
            }
            let item2 = type_flags_def.get_item(j).unwrap();
            let ident1 = item1.ident();
            let ident2 = item2.ident();
            let tokens = quote!(
                impl ::kxos_typeflags_util::SameAs<#ident1> for #ident2 {
                    type Output = ::kxos_typeflags_util::False;
                }
            );
            all_tokens.append_all(tokens);
        }
    }
    all_tokens
}

/// export the declarive macro
pub fn export_declarive_macro(type_flags_def: &TypeFlagDef, flag_sets: &[FlagSet]) -> TokenStream {
    let macro_ident = type_flags_def.trait_ident();
    let macro_item_tokens = flag_sets
        .iter()
        .map(|flag_set| flag_set.macro_item_tokens())
        .fold(Vec::new(), |mut left, mut new_item| {
            left.append(&mut new_item);
            left
        });

    let tokens = quote!(
        #[macro_export]
        macro_rules! #macro_ident {
            #(#macro_item_tokens);*
        }
    );

    tokens
}
