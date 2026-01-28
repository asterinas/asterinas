// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;

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
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("zerocopy", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("FromBytes", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));

                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("zerocopy", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("IntoBytes", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));

                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("zerocopy", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("Immutable", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));

                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new("zerocopy", Span::call_site())));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
                new_derives.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
                new_derives.push(TokenTree::Ident(Ident::new(
                    "KnownLayout",
                    Span::call_site(),
                )));

                // Check if next token is a comma, if so skip it
                i += 1;
                if i < tokens.len()
                    && let TokenTree::Punct(punct) = &tokens[i]
                    && punct.as_char() == ','
                {
                    // Add comma and skip the next comma token
                    new_derives.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
                    i += 1;
                    continue;
                }
                continue;
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
