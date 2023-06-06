use std::{collections::VecDeque, f32::consts::E};

use proc_macro2::{Ident, Span, TokenStream, TokenTree};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{
    fold::{fold_block, fold_signature, Fold},
    parse::Parse,
    parse_quote,
    punctuated::Punctuated,
    GenericArgument, GenericParam, ItemImpl, Meta, PathArguments, PathSegment, Token, Type,
    TypeParam, TypePath, WhereClause,
};

#[derive(Debug)]
pub struct CapType {
    type_name: Ident,
}

impl Parse for CapType {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let type_name: Ident = input.parse()?;
        Ok(Self { type_name })
    }
}

pub fn expand_static_cap(item_impl: ItemImpl, mut cap_type: CapType) -> TokenStream {
    let new_impl_item = cap_type.fold_item_impl(item_impl);
    quote!(#new_impl_item)
}

impl Fold for CapType {
    fn fold_type(&mut self, type_: Type) -> Type {
        if let Some(new_type) = try_replace_cap_type(type_.clone(), self.type_name.clone()) {
            return new_type;
        }
        if let Type::Path(type_path) = type_ {
            if let Some(new_type_path) = try_replace_type_from_typeflags(type_path.clone()) {
                return Type::Path(new_type_path);
            }
            let type_path = self.fold_type_path(type_path);
            return Type::Path(type_path);
        }
        return type_;
    }

    fn fold_generics(&mut self, mut generics: syn::Generics) -> syn::Generics {
        let syn::Generics {
            params,
            where_clause,
            ..
        } = &mut generics;
        let comma = parse_quote!(,);
        for param in params.iter_mut() {
            let bounds = if let GenericParam::Type(TypeParam { ident, bounds, .. }) = param {
                if *ident == self.type_name && bounds.len() == 1 {
                    bounds
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let bound = bounds.first().unwrap().clone();
            bounds.clear();
            let ident = self.type_name.clone();
            let where_prediacate = parse_quote!(::typeflags_util::TypeWrapper<#ident>: #bound);
            if let Some(where_clause) = where_clause.as_mut() {
                if !where_clause.predicates.empty_or_trailing() {
                    where_clause.predicates.push_punct(comma);
                }
                where_clause.predicates.push(where_prediacate);
            } else {
                let mut predicates = Punctuated::new();
                predicates.push(where_prediacate);
                let where_token = parse_quote!(where);
                let new_where_clause = WhereClause {
                    where_token,
                    predicates,
                };
                *where_clause = Some(new_where_clause);
            }
        }

        return generics;
    }

    fn fold_impl_item_fn(&mut self, mut item_fn: syn::ImplItemFn) -> syn::ImplItemFn {
        for attribute in &mut item_fn.attrs {
            let meta_list = if let Meta::List(meta_list) = &mut attribute.meta {
                meta_list
            } else {
                continue;
            };
            let segments = &meta_list.path.segments;
            if segments.len() != 1 {
                continue;
            }
            let first_segment = segments.first().unwrap().ident.to_string();
            if first_segment.as_str() != "require" {
                continue;
            }
            let mut tokens: VecDeque<_> = meta_list.tokens.clone().into_iter().collect();
            let ident = if let Some(TokenTree::Ident(ident)) = tokens.pop_front() && ident == self.type_name {
                ident
            } else {
                continue;
            };
            let new_tokens = {
                let required_type: Type = parse_quote!(::typeflags_util::TypeWrapper<#ident>);
                let mut new_tokens = required_type.into_token_stream();
                new_tokens.append_all(tokens.into_iter());
                new_tokens
            };
            meta_list.tokens = new_tokens;
        }
        item_fn.sig = fold_signature(self, item_fn.sig);
        item_fn.block = fold_block(self, item_fn.block);
        return item_fn;
    }

    fn fold_expr_path(&mut self, mut expr_path: syn::ExprPath) -> syn::ExprPath {
        if expr_path.path.segments.len() <= 0 {
            return expr_path;
        }
        let path = &mut expr_path.path;
        let leading_ident = path.segments.first().unwrap().ident.clone();
        if leading_ident != self.type_name {
            return expr_path;
        }
        let path_sep: Token!(::) = parse_quote!(::);
        path.leading_colon = Some(path_sep.clone());
        let segments = {
            let mut segments = Punctuated::new();
            let segment = {
                let ident = Ident::new("typeflags_util", Span::call_site());
                PathSegment {
                    ident,
                    arguments: PathArguments::None,
                }
            };
            segments.push_value(segment);
            segments.push_punct(path_sep.clone());
            let segment = {
                let ident = Ident::new("TypeWrapper", Span::call_site());
                let arguments = parse_quote!(::<#leading_ident>);
                PathSegment {
                    ident,
                    arguments: PathArguments::AngleBracketed(arguments),
                }
            };
            segments.push_value(segment);
            let mut iter = path.segments.iter();
            iter.advance_by(1).unwrap();
            for segment in iter {
                segments.push_punct(path_sep.clone());
                segments.push_value(segment.clone());
            }
            segments
        };
        expr_path.path.segments = segments;
        expr_path
    }
}

fn try_replace_type(old_type: Type, type_name: Ident) -> Option<Type> {
    let mut type_path = if let Type::Path(type_path) = old_type.clone() {
        type_path
    } else {
        return None;
    };
    // FIXME: we currently assume `type_name` appears only once
    for segment in type_path.path.segments.iter_mut() {
        let arguments = if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
            arguments
        } else {
            continue;
        };
        for argument in &mut arguments.args {
            let old_type = if let GenericArgument::Type(type_) = &argument {
                type_.clone()
            } else {
                continue;
            };
            if let Some(new_type) = try_replace_cap_type(old_type, type_name.clone()) {
                *argument = GenericArgument::Type(new_type);
                return Some(Type::Path(type_path));
            }
        }
    }
    return None;
}

fn try_replace_cap_type(old_type: Type, type_name: Ident) -> Option<Type> {
    let type_path = if let Type::Path(type_path) = old_type {
        if type_path.path.segments.len() == 1 {
            type_path
        } else {
            return None;
        }
    } else {
        return None;
    };
    let segment = type_path.path.segments.first().unwrap();
    if segment.ident == type_name {
        let new_type = parse_quote!(::typeflags_util::TypeWrapper<#type_name>);
        return Some(new_type);
    }
    return None;
}

fn try_replace_type_from_typeflags(old_type_path: TypePath) -> Option<TypePath> {
    const TYPES_FROM_TYPEFLAGS: &[&str] = &["SetExtendOp"];
    if old_type_path.path.segments.len() == 0 {
        return None;
    }
    let leading_ident = old_type_path
        .path
        .segments
        .first()
        .unwrap()
        .ident
        .to_string();
    if let Some(_) = TYPES_FROM_TYPEFLAGS
        .iter()
        .position(|type_from_typeflags| *type_from_typeflags == leading_ident.as_str())
    {
        let typepath = parse_quote!(::typeflags_util::TypeWrapper<#old_type_path>);
        return Some(typepath);
    }
    return None;
}
