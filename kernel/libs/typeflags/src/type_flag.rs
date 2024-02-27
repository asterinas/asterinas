// SPDX-License-Identifier: MPL-2.0

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    braced,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Attribute, Expr, Ident, Token, Type, Visibility,
};

/// The content inside typeflag macro
pub struct TypeFlagDef {
    attributes: Vec<Attribute>,
    ident: Ident,
    vis: Visibility,
    type_: Type,
    items: Punctuated<TypeFlagItem, Token![;]>,
}

/// struct item inside typeflag macro
#[derive(Clone)]
pub struct TypeFlagItem {
    attributes: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    value: Expr,
}

impl Parse for TypeFlagDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let attributes = Attribute::parse_outer(input)?;
        let vis: Visibility = input.parse()?;
        let _: Token![trait] = input.parse()?;
        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let type_: Type = input.parse()?;
        // read content inside brace
        let content;
        let _ = braced!(content in input);
        let items = content.parse_terminated(TypeFlagItem::parse)?;

        let res = TypeFlagDef {
            attributes,
            ident,
            vis,
            type_,
            items,
        };

        Ok(res)
    }
}

impl Parse for TypeFlagItem {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attributes = Attribute::parse_outer(input)?;
        let vis: Visibility = input.parse()?;
        let _: Token![struct] = input.parse()?;
        let ident: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;
        let value: Expr = input.parse()?;
        let res = TypeFlagItem {
            attributes,
            vis,
            ident,
            value,
        };
        Ok(res)
    }
}

impl TypeFlagDef {
    /// tokens to define the trait
    pub fn trait_def_tokens(&self) -> TokenStream {
        let attributes = self.attributes.clone();
        let vis = self.vis.clone();
        let ident = self.ident.clone();
        let type_ = self.type_.clone();
        quote!(
            #(#attributes)*
            #vis trait #ident : Sync + Send + Copy + Clone{
                const BITS: #type_;

                fn new() -> Self;
            }

        )
    }

    /// tokens to define all structs
    pub fn items_def_tokens(&self) -> Vec<TokenStream> {
        self.items
            .iter()
            .map(|type_flag_item| type_flag_item.item_def_tokens())
            .collect()
    }

    /// return the items iter
    pub fn items_iter(&self) -> syn::punctuated::Iter<TypeFlagItem> {
        self.items.iter()
    }

    /// the number of items
    pub fn item_num(&self) -> usize {
        self.items.len()
    }

    /// get item at specific position
    pub fn get_item(&self, index: usize) -> Option<TypeFlagItem> {
        self.items.iter().nth(index).cloned()
    }

    /// the trait ident
    pub fn trait_ident(&self) -> Ident {
        self.ident.clone()
    }

    /// the val type
    pub fn val_type(&self) -> Type {
        self.type_.clone()
    }
}

impl TypeFlagItem {
    /// the token stream to define such item
    fn item_def_tokens(&self) -> TokenStream {
        let attributes = self.attributes.clone();
        let vis = self.vis.clone();
        let ident = self.ident.clone();
        quote!(
            #(#attributes)*
            #[derive(Copy, Clone, Debug)]
            #vis struct #ident {}
        )
    }

    /// the item's ident
    pub fn ident(&self) -> Ident {
        self.ident.clone()
    }

    /// the expression of the items's value
    pub fn val(&self) -> Expr {
        self.value.clone()
    }
}

impl TypeFlagDef {
    /// Debug use. Print all item's name.
    pub fn debug(&self) {
        println!("{}", self.ident);
        for type_flag_item in &self.items {
            println!("{}", type_flag_item.ident);
        }
    }
}
