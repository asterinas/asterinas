// SPDX-License-Identifier: MPL-2.0

//! expand the require macro

use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{
    fold::Fold, parse::Parse, parse_quote, punctuated::Punctuated, token::Comma, GenericParam,
    Generics, Token, Type, WhereClause,
};

use super::require_item::RequireItem;

pub struct RequireAttr {
    type_set: Type,
    required_types: Punctuated<Ident, Token![|]>,
}

impl Parse for RequireAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let type_set: Type = input.parse()?;
        let _: Token![>] = input.parse()?;
        let required_types = input.parse_terminated(Ident::parse)?;
        Ok(RequireAttr {
            type_set,
            required_types,
        })
    }
}

impl Fold for RequireAttr {
    fn fold_generics(&mut self, i: Generics) -> Generics {
        let Generics {
            lt_token,
            params,
            gt_token,
            where_clause,
        } = i;

        let mut new_where_clause = where_clause;
        for required_type in self.required_types.iter() {
            // If required type is a generic param, the required type should be a type set, we use SetInclude.
            // Otherwise, we use SetContain.
            if is_generic_param(required_type.clone(), &params) {
                new_where_clause = Some(set_include_where_clause(
                    self,
                    required_type.clone(),
                    new_where_clause,
                ));
            } else {
                new_where_clause = Some(set_contain_where_clause(
                    self,
                    required_type.clone(),
                    new_where_clause,
                ));
            }
        }

        Generics {
            lt_token,
            params,
            gt_token,
            where_clause: new_where_clause,
        }
    }
}

pub fn expand_require(item: RequireItem, mut require_attr: RequireAttr) -> TokenStream {
    match item {
        RequireItem::Impl(item_impl) => {
            let new_item_impl = require_attr.fold_item_impl(item_impl);
            quote!(
                #[allow(clippy::multiple_bound_locations)]
                #new_item_impl
            )
        }
        RequireItem::Fn(item_fn) => {
            let new_item_fn = require_attr.fold_item_fn(item_fn);
            quote!(
                #[allow(clippy::multiple_bound_locations)]
                #new_item_fn
            )
        }
    }
}

fn is_generic_param(ident: Ident, generic_params: &Punctuated<GenericParam, Comma>) -> bool {
    for generic_param in generic_params {
        match generic_param {
            GenericParam::Type(type_param) => {
                let type_param_ident = type_param.ident.clone();
                if ident == type_param_ident {
                    return true;
                }
            }
            GenericParam::Const(const_param) => {
                let const_param_ident = const_param.ident.clone();
                if const_param_ident == ident {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn set_contain_where_clause(
    require_attr: &RequireAttr,
    required_type: Ident,
    old_where_clause: Option<WhereClause>,
) -> WhereClause {
    let type_set = require_attr.type_set.clone();
    let mut where_predicates = Vec::new();
    where_predicates.push(parse_quote!(
        #type_set: ::typeflags_util::SetContain<#required_type>
    ));
    where_predicates.push(parse_quote!(
        ::typeflags_util::SetContainOp<#type_set, #required_type>: ::typeflags_util::IsTrue
    ));

    let comma = parse_quote!(,);
    match old_where_clause {
        None => {
            let where_token = parse_quote!(where);
            let mut predicates = Punctuated::new();
            for predicate in where_predicates {
                if !predicates.empty_or_trailing() {
                    predicates.push_punct(comma);
                }
                predicates.push_value(predicate);
            }
            WhereClause {
                where_token,
                predicates,
            }
        }
        Some(old_where_clause) => {
            let WhereClause {
                where_token,
                mut predicates,
            } = old_where_clause;

            for predicate in where_predicates {
                if !predicates.empty_or_trailing() {
                    predicates.push_punct(comma);
                }
                predicates.push_value(predicate);
            }
            WhereClause {
                where_token,
                predicates,
            }
        }
    }
}

/// generate a where clause to constraint the type set with another type set
fn set_include_where_clause(
    require_attr: &RequireAttr,
    required_type_set: Ident,
    old_where_clause: Option<WhereClause>,
) -> WhereClause {
    let type_set = require_attr.type_set.clone();
    let comma = parse_quote!(,);

    let mut additional_where_prediates = Vec::new();
    additional_where_prediates.push(parse_quote!(
        #required_type_set: ::typeflags_util::Set
    ));
    additional_where_prediates.push(parse_quote!(
        #type_set: ::typeflags_util::SetInclude<#required_type_set>
    ));
    additional_where_prediates.push(parse_quote!(
        ::typeflags_util::SetIncludeOp<#type_set, #required_type_set>: ::typeflags_util::IsTrue
    ));

    match old_where_clause {
        None => {
            let where_token = parse_quote!(where);
            let mut predicates = Punctuated::new();
            for predicate in additional_where_prediates.into_iter() {
                if !predicates.empty_or_trailing() {
                    predicates.push_punct(comma);
                }
                predicates.push_value(predicate);
            }
            WhereClause {
                where_token,
                predicates,
            }
        }
        Some(old_where_clause) => {
            let WhereClause {
                where_token,
                mut predicates,
            } = old_where_clause;
            for predicate in additional_where_prediates.into_iter() {
                if !predicates.empty_or_trailing() {
                    predicates.push_punct(comma);
                }
                predicates.push_value(predicate);
            }
            WhereClause {
                where_token,
                predicates,
            }
        }
    }
}
