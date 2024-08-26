// SPDX-License-Identifier: MPL-2.0

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{quote, TokenStreamExt};
use syn::Expr;

use crate::type_flag::TypeFlagDef;

const EMPTY_SET_NAME: &str = "::typeflags_util::Nil";
const SET_NAME: &str = "::typeflags_util::Cons";

/// A flagSet represent the combination of different flag item.
/// e.g. [Read, Write], [Read], [] are all flag sets.
/// The order of flagItem does not matters. So flag sets with same sets of items should be viewed as the same set.
#[derive(Debug)]
pub struct FlagSet {
    items: Vec<FlagItem>,
}

impl FlagSet {
    /// create a new empty flag set
    pub fn new() -> Self {
        FlagSet { items: Vec::new() }
    }

    /// add a flag item
    pub fn push_item(&mut self, flag_item: FlagItem) {
        self.items.push(flag_item);
    }

    /// the tokens represents the flag set type name
    pub fn type_name_tokens(&self) -> TokenStream {
        let mut res = quote!(::typeflags_util::Nil);

        for item in self.items.iter() {
            let ident = item.ident.clone();

            // insert new item as head
            let new_res = quote! {
                ::typeflags_util::Cons<#ident, #res>
            };
            res = new_res;
        }

        res
    }

    /// the number of items
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// the tokens to impl main trait for the current flagset
    pub fn impl_main_trait_tokens(&self, type_flags_def: &TypeFlagDef) -> TokenStream {
        let trait_ident = type_flags_def.trait_ident();
        let name = self.type_name_tokens();
        let mut all_tokens = quote! (
            impl #trait_ident for #name
        );
        all_tokens.append_all(self.inner_tokens(type_flags_def));
        all_tokens
    }

    /// the impl main trait inner content
    fn inner_tokens(&self, type_flags_def: &TypeFlagDef) -> TokenStream {
        let ty = type_flags_def.val_type();
        let item_vals = self.items.iter().map(|item| item.val.clone());

        // quote seems unable to use string for types.
        // So we hardcode all types here.
        if item_vals.len() == 0 {
            quote!(
                {
                    const BITS: #ty = 0 ;
                    fn new() -> Self {
                        ::typeflags_util::Nil
                    }
                }
            )
        } else {
            quote!(
                {
                    const BITS: #ty = #(#item_vals)|* ;
                    fn new() -> Self {
                        ::typeflags_util::Cons::new()
                    }
                }
            )
        }
    }

    pub fn contains_type(&self, type_ident: &Ident) -> bool {
        let type_name = type_ident.to_string();
        self.items.iter().any(|item| item.ident == type_name)
    }

    pub fn contains_set(&self, other_set: &FlagSet) -> bool {
        for item in &other_set.items {
            if !self.contains_type(&item.ident) {
                return false;
            }
        }
        true
    }

    /// The token stream inside macro definition. We will generate a token stream for each permutation of items
    /// since the user may use arbitrary order of items in macro.
    pub fn macro_item_tokens(&self) -> Vec<TokenStream> {
        let res_type = self.type_name_tokens();
        // We first calculate the full permutations,
        let item_permutations = self.items.iter().permutations(self.items.len());
        item_permutations
            .map(|flag_items| {
                let item_names = flag_items
                    .into_iter()
                    .map(|flag_item| flag_item.ident.clone())
                    .collect::<Vec<_>>();
                quote! (
                    (#(#item_names),*) => { #res_type }
                )
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct FlagItem {
    /// the user provided name
    ident: Ident,
    /// the user-provided val
    val: Expr,
}

impl core::fmt::Debug for FlagItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlagItem")
            .field("ident", &self.ident.to_string())
            .finish()
    }
}

/// generate all possible flag sets
pub fn generate_flag_sets(type_flag_def: &TypeFlagDef) -> Vec<FlagSet> {
    let flag_items = type_flag_def
        .items_iter()
        .map(|type_flag_item| {
            let ident = type_flag_item.ident();
            let val = type_flag_item.val();
            FlagItem { ident, val }
        })
        .collect::<Vec<_>>();
    let flag_item_num = flag_items.len();
    let limit = 1 << flag_items.len();
    let mut res = Vec::with_capacity(limit);

    for i in 0..limit {
        let mut flag_set = FlagSet::new();
        for (j, item_j) in flag_items.iter().enumerate().take(flag_item_num) {
            if (i >> j) & 0x1 == 1usize {
                flag_set.push_item(item_j.clone());
            }
        }
        res.push(flag_set);
    }

    res
}
