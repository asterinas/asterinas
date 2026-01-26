// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;

fn push_zerocopy_derive(
    derives: &mut Vec<proc_macro::TokenTree>,
    ident: &str,
    trailing_comma: bool,
) {
    use proc_macro::{Ident, Punct, Spacing, Span, TokenTree};

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
    use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenTree};

    // Process the derive attributes
    let mut new_derives = Vec::new();
    let tokens: Vec<TokenTree> = attrs.into_iter().collect();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(ident) if ident.to_string() == "Pod" => {
                // Replace Pod with zerocopy traits
                push_zerocopy_derive(&mut new_derives, "FromBytes", true);
                push_zerocopy_derive(&mut new_derives, "IntoBytes", true);
                push_zerocopy_derive(&mut new_derives, "Immutable", true);
                push_zerocopy_derive(&mut new_derives, "KnownLayout", false);

                i += 1;
            }
            _ => {
                new_derives.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    // Build the output: #[::core::prelude::v1::derive(...)] + input
    let mut output = TokenStream::new();

    // Add the derive attribute
    output.extend(vec![
        TokenTree::Punct(Punct::new('#', Spacing::Alone)),
        TokenTree::Group(Group::new(Delimiter::Bracket, {
            let mut attr_tokens = TokenStream::new();
            // ::core::prelude::v1::derive
            attr_tokens.extend(vec![
                TokenTree::Punct(Punct::new(':', Spacing::Joint)),
                TokenTree::Punct(Punct::new(':', Spacing::Alone)),
                TokenTree::Ident(Ident::new("core", Span::call_site())),
                TokenTree::Punct(Punct::new(':', Spacing::Joint)),
                TokenTree::Punct(Punct::new(':', Spacing::Alone)),
                TokenTree::Ident(Ident::new("prelude", Span::call_site())),
                TokenTree::Punct(Punct::new(':', Spacing::Joint)),
                TokenTree::Punct(Punct::new(':', Spacing::Alone)),
                TokenTree::Ident(Ident::new("v1", Span::call_site())),
                TokenTree::Punct(Punct::new(':', Spacing::Joint)),
                TokenTree::Punct(Punct::new(':', Spacing::Alone)),
                TokenTree::Ident(Ident::new("derive", Span::call_site())),
            ]);
            // Add the derives in parentheses
            attr_tokens.extend(vec![TokenTree::Group(Group::new(
                Delimiter::Parenthesis,
                new_derives.into_iter().collect(),
            ))]);
            attr_tokens
        })),
    ]);

    // Add the original input
    output.extend(input);

    output
}
