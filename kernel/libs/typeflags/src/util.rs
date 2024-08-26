// SPDX-License-Identifier: MPL-2.0

use proc_macro2::{Ident, TokenStream};
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

    let flag_sets = generate_flag_sets(type_flags_def);
    flag_sets.iter().for_each(|flag_set| {
        let impl_main_trait_tokens = flag_set.impl_main_trait_tokens(type_flags_def);
        all_tokens.append_all(impl_main_trait_tokens);
    });

    let impl_set_intend_tokens = impl_set_extend(type_flags_def, &flag_sets);
    all_tokens.append_all(impl_set_intend_tokens);

    let export_declarive_macro_tokens = export_declarive_macro(type_flags_def, &flag_sets);
    all_tokens.append_all(export_declarive_macro_tokens);

    all_tokens
}

/// import crate typeflags_util
pub fn import_util() -> TokenStream {
    quote!(
        #[macro_use]
        use ::typeflags_util::*;
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
                impl ::typeflags_util::SameAs<#ident1> for #ident2 {
                    type Output = ::typeflags_util::False;
                }
            );
            all_tokens.append_all(tokens);
        }
    }
    all_tokens
}

pub fn impl_set_extend(type_flags_def: &TypeFlagDef, flag_sets: &[FlagSet]) -> TokenStream {
    let mut all_tokens = TokenStream::new();
    let type_idents: Vec<_> = type_flags_def
        .items_iter()
        .map(|type_flag_item| type_flag_item.ident())
        .collect();

    for flag_set in flag_sets {
        // We don't need to impl set extend trait for Nil
        if flag_set.len() == 0 {
            continue;
        }
        for type_ident in &type_idents {
            let type_ident = type_ident.clone();
            let flag_set_tokens = flag_set.type_name_tokens();
            if flag_set.contains_type(&type_ident) {
                // the flagset contains the type
                let impl_extend_tokens = quote!(
                    impl ::typeflags_util::SetExtend<#type_ident> for #flag_set_tokens {
                        type Output = #flag_set_tokens;
                    }
                );
                all_tokens.append_all(impl_extend_tokens)
            } else {
                // the flagset does not contains the type
                let output_set = extent_one_type(&type_ident, flag_set, flag_sets).unwrap();
                let output_set_tokens = output_set.type_name_tokens();
                let impl_extend_tokens = quote!(
                    impl ::typeflags_util::SetExtend<#type_ident> for #flag_set_tokens {
                        type Output = #output_set_tokens;
                    }
                );
                all_tokens.append_all(impl_extend_tokens);
            }
        }
    }

    all_tokens
}

fn extent_one_type<'a>(
    type_ident: &Ident,
    flag_set: &'a FlagSet,
    sets: &'a [FlagSet],
) -> Option<&'a FlagSet> {
    sets.iter().find(|bigger_set| {
        bigger_set.contains_type(type_ident) && bigger_set.contains_set(flag_set)
    })
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
