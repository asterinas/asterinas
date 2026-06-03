// SPDX-License-Identifier: MPL-2.0

use quote::ToTokens;
use syn::visit_mut::VisitMut;

use crate::args::AddArguments;

fn expand(file: &str, pat: &str) {
    let mut args = AddArguments::test_new("procfs", "crate::fs::procfs");
    let mut file = syn::parse_str(file).unwrap();
    args.visit_file_mut(&mut file);

    // Plain token stream to string will generate tokens with whitespaces around,
    // causing string search failure. So format tokens pretty as normal code.
    let expansion = prettyplease::unparse(&file);
    assert!(
        expansion.contains(pat),
        "`{pat}` must be in:\n`{expansion}`"
    );
}

#[test]
fn fn_() {
    let file = "pub(in procfs) fn foo() {}";
    expand(file, "pub(in crate::fs::procfs) fn foo");
}

#[test]
fn struct_() {
    let file = "pub(in procfs) struct Data { field: () }";
    expand(file, "pub(in crate::fs::procfs) struct Data");
}

#[test]
fn field() {
    let file = "pub struct Data { pub(in procfs) field: () }";
    expand(file, "pub(in crate::fs::procfs) field");
}

#[test]
fn inherent_fn() {
    let file = "pub struct S; impl S { pub(in procfs) fn nested(&self) {} }";
    expand(file, "pub(in crate::fs::procfs) fn nested");
}

#[test]
fn macro2() {
    let mut args = AddArguments::test_new("procfs", "crate::fs::procfs");
    let mut file = syn::parse_str("pub(in procfs) macro bar() {}").unwrap();
    args.visit_file_mut(&mut file);

    // prettyplease panics on verbatim tokens, so back to_string.
    let expansion = file.into_token_stream().to_string();
    let pat = "pub (in crate :: fs :: procfs) macro bar";
    assert!(
        expansion.contains(pat),
        "`{pat}` must be in:\n`{expansion}`"
    );
}
