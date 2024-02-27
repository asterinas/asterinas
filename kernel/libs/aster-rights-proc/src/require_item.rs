// SPDX-License-Identifier: MPL-2.0

use syn::{parse::Parse, ItemFn, ItemImpl, Token};

pub enum RequireItem {
    Impl(ItemImpl),
    Fn(ItemFn),
}

impl Parse for RequireItem {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let token = input.lookahead1();
        if token.peek(Token!(impl)) {
            // FIXME: is there any possible token before impl?
            input.parse().map(RequireItem::Impl)
        } else {
            input.parse().map(RequireItem::Fn)
        }
    }
}
