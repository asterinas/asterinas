// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;
use quote::quote;

fn push_zerocopy_derive(
    derives: &mut Vec<proc_macro2::TokenTree>,
    ident: &str,
    trailing_comma: bool,
) {
    use proc_macro2::{Ident, Punct, Spacing, Span, TokenTree};

    derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
    derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
    derives.push(TokenTree::Ident(Ident::new("zerocopy", Span::call_site())));
    derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
    derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
    derives.push(TokenTree::Ident(Ident::new(ident, Span::call_site())));
    if trailing_comma {
        derives.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
    }
}

pub fn expand_derive(attrs: TokenStream, input: TokenStream) -> TokenStream {
    use proc_macro2::TokenTree;

    // Process the derive attributes
    let mut new_derives = Vec::new();
    let attr_tokens = proc_macro2::TokenStream::from(attrs);
    for token in attr_tokens.into_iter() {
        match token {
            TokenTree::Ident(ident) if ident == "Pod" => {
                // Replace Pod with zerocopy traits
                push_zerocopy_derive(&mut new_derives, "FromBytes", true);
                push_zerocopy_derive(&mut new_derives, "IntoBytes", true);
                push_zerocopy_derive(&mut new_derives, "Immutable", true);
                push_zerocopy_derive(&mut new_derives, "KnownLayout", false);
            }
            _ => {
                new_derives.push(token);
            }
        }
    }

    // Build the output: #[::core::prelude::v1::derive(...)] + input
    let input2: proc_macro2::TokenStream = input.into();
    quote!(
        #[::core::prelude::v1::derive(#(#new_derives)*)]
        #input2
    )
    .into()
}
