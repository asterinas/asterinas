// SPDX-License-Identifier: MPL-2.0

use std::collections::BTreeMap;

use proc_macro2::{Group, Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{parse::Parse, punctuated::Punctuated, *};

/// Represents a single argument in the `#[add(...)]` attribute.
/// Either a simple identifier or an override with an explicit path.
enum Argument {
    Single(Ident),
    Override(Ident, Path),
}

/// Parses `Argument` from token stream.
/// Accepts either a single identifier or `ident = path` format.
impl Parse for Argument {
    fn parse(input: parse::ParseStream) -> Result<Self> {
        Ok(if input.peek2(Token![=]) {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;
            let path: Path = input.parse()?;
            Argument::Override(ident, path)
        } else {
            let ident: Ident = input.parse()?;
            Argument::Single(ident)
        })
    }
}

/// Holds the parsed arguments from `#[add(...)]`.
/// Maps each identifier to its corresponding path.
pub struct AddArguments {
    args: BTreeMap<Ident, Path>,
}

/// Parses the `#[add(...)]` attribute content.
/// Expects a comma-separated list of identifiers, optionally with path overrides.
impl Parse for AddArguments {
    fn parse(input: parse::ParseStream) -> Result<Self> {
        // Parse multiple arguments.
        let args = Punctuated::<Argument, Token![,]>::parse_terminated(input)?;

        // Default module path inferred from file path.
        let path = ExpandedPath::new();

        Ok(AddArguments {
            args: args
                .into_iter()
                .map(|arg| match arg {
                    Argument::Single(ident) => {
                        let Some(tokens) = path.to_syn_path(&ident) else {
                            panic!(
                                "The path `{}` doesn't contain `{ident}`. \
                                 Please choose a correct short module name.",
                                path.segment.join("::")
                            )
                        };
                        (ident, tokens)
                    }
                    Argument::Override(ident, path) => (ident, path.clone()),
                })
                .collect(),
        })
    }
}

/// Implements VisitMut to transform visibility paths in AST nodes.
impl visit_mut::VisitMut for AddArguments {
    fn visit_visibility_mut(&mut self, vis: &mut Visibility) {
        self.replace_restricted_vis_path(vis);
    }

    fn visit_item_mut(&mut self, item: &mut Item) {
        if let Item::Verbatim(ts) = item {
            // Syn doesn't support parsing `pub(in path) macro` yet.
            self.replace_verbatim_vis_path(ts);
            return;
        }
        visit_mut::visit_item_mut(self, item);
    }
}

/// Provides methods for replacing short visibility paths with full paths.
impl AddArguments {
    /// Replaces `pub(in subsystem)` with `pub(in crate::to::subsystem)`.
    /// Only affects visibility restricted to identifiers registered in `self.args`.
    fn replace_restricted_vis_path(&self, vis: &mut Visibility) {
        if let Visibility::Restricted(vis) = vis
            && let Some(input) = vis.path.get_ident()
            && let Some(path) = self.args.get(input)
        {
            vis.path = Box::clone_from_ref(path);
        }
    }

    /// Parses and replaces visibility paths in verbatim token streams.
    /// Handles `pub(in ident)` syntax that syn cannot parse normally.
    fn replace_verbatim_vis_path(&self, ts: &mut TokenStream) {
        let mut v_tt: Vec<TokenTree> = ts.clone().into_iter().collect();
        let mut iter = v_tt.iter_mut();
        if let Some(TokenTree::Ident(ident)) = iter.next()
            && ident == "pub"
            && let Some(TokenTree::Group(group)) = iter.next()
        {
            let mut new_stream = TokenStream::new();
            let mut stream = group.stream().into_iter();
            if let Some(in_) = stream.next()
                && let TokenTree::Ident(ident) = &in_
                && ident == "in"
            {
                new_stream.extend([in_]);

                let path_stream = stream.collect::<TokenStream>();
                if let Ok(input) = parse2::<Ident>(path_stream)
                    && let Some(path) = self.args.get(&input)
                {
                    path.to_tokens(&mut new_stream);
                    // Replace the group's token stream directly.
                    *group = Group::new(group.delimiter(), new_stream);
                }
            }
            // Update token stream with the modified v_tt since the original ts doesn't apply the change.
            *ts = TokenStream::from_iter(v_tt);
        }
    }

    #[cfg(test)]
    pub fn test_new(ident: &str, path: &str) -> Self {
        let mut args = BTreeMap::new();
        let path = parse_str(path).unwrap();
        args.insert(Ident::new(ident, Span::call_site()), path);
        AddArguments { args }
    }
}

/// Represents the full module path derived from the source file location.
/// Used to replace short visibility paths with properly qualified paths.
struct ExpandedPath {
    /// Module path segments starting from `crate`.
    segment: Vec<String>,
    /// Span for maintaining original source location in generated tokens.
    callsite_span: Span,
}

impl ExpandedPath {
    /// Constructs the full module path based on the source file location.
    /// The path starts from `crate` and follows the directory structure.
    /// For example, if the attribute is in `a/src/procfs.rs`, this function returns
    /// `crate::procfs`; if in `a/src/fs/procfs/mod.rs`, returns `crate::fs::procfs`.
    fn new() -> Self {
        let callsite_span = Span::call_site();
        let Some(local_path) = callsite_span.local_file() else {
            panic!("Unknown local file path to call site span {callsite_span:?}.");
        };
        let Ok(local_path) = local_path.canonicalize() else {
            panic!("Unable to canonicalize {local_path:?}.")
        };

        let prefix = {
            let dir = std::env::var("CARGO_MANIFEST_DIR").expect("Failed to get manifest dir.");
            std::path::PathBuf::from(dir).join("src")
        };

        // Strip `$CARGO_MANIFEST_DIR/src/`.
        let Ok(module_path) = local_path.strip_prefix(&prefix) else {
            panic!("{prefix:?} must be a prefix of {local_path:?}.")
        };

        // Strip `/mod.rs` from `parent/child/mod.rs` for child module.
        let module_path = if module_path.file_name() == Some("mod.rs".as_ref()) {
            module_path.parent().unwrap()
        } else {
            module_path
        };
        // Strip `.rs` from `parent/child.rs` for child module.
        let module_path = module_path.with_extension("");

        ExpandedPath {
            segment: std::iter::once("crate")
                .chain(module_path.iter().map(|m| m.to_str().unwrap()))
                .map(String::from)
                .collect(),
            callsite_span,
        }
    }

    /// Generates a `Path` from `crate` up to and including the segment matching `end`.
    /// Returns `None` if `end` is not found in the module path.
    fn to_syn_path(&self, end: &Ident) -> Option<Path> {
        let pos = self.segment.iter().rposition(|seg| end == seg.as_str())?;
        Some(Path {
            leading_colon: None,
            segments: self.segment[..pos + 1]
                .iter()
                .map(|s| PathSegment::from(Ident::new(s, self.callsite_span)))
                .collect(),
        })
    }
}
