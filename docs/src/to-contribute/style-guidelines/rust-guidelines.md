# Rust Guidelines

## API Documentation Guidelines

API documentation describes the meanings and usage of APIs,
and will be rendered into web pages by rustdoc.

It is necessary to add documentation to all public APIs,
including crates, modules, structs, traits, functions, macros, and more.
The use of the `#[warn(missing_docs)]` lint enforces this rule.

Asterinas adheres to the API style guidelines of the Rust community.
The recommended API documentation style can be found at
[how-to-write-documentation](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html).
