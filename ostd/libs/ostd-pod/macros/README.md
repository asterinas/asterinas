<!--
To promote a "single source of truth", the content of `README.md` is also included in `lib.rs`
as the crate-level documentation. So when writing this README, bear in mind that its content
should be recognized correctly by both a Markdown renderer and the rustdoc tool.
-->

# ostd-pod-macros

Procedural macros for the [ostd-pod] crate.

This crate provides procedural macros to simplify working with Plain Old Data (POD) types. 
It exports two main macros:

- `#[derive(Pod)]`: An attribute macro that expands into the underlying `zerocopy` traits
- `#[pod_union]`: An attribute macro that makes unions safe to use as POD types

## The `derive` Macro

The `#[derive(Pod)]` macro is a convenience wrapper that automatically derives the required [zerocopy] traits for POD types. 

Unlike typical derive procedural macros, `derive` in this crate is actually an **attribute** macro that works by shadowing [`::core::prelude::v1::derive`].

## The `pod_union` Macro

The `#[pod_union]` attribute macro enables safe usage of unions as POD types. It automatically:

- Derives the necessary [zerocopy] traits
- Generates safe initializer and accessor methods for each union field
- Enforces `#[repr(C)]` layout
- Ensures all fields are POD types

### Generated Initializer and Accessor Methods

For each field `foo` in the union, the macro generates:

- `fn new_foo(value: FieldType) -> Self`: Constructs an instance from the field
- `fn foo(&self) -> &FieldType`: Returns a reference to the field
- `fn foo_mut(&mut self) -> &mut FieldType`: Returns a mutable reference to the field

These methods use `zerocopy`'s safe byte conversion methods, avoiding unsafe code.

For detailed usage examples, see the crate [ostd-pod] documentation.

## License

This project is licensed under MPL-2.0.

<!--
External links.
-->
[ostd-pod]: https://docs.rs/ostd-pod/
[zerocopy]: https://docs.rs/zerocopy/
[`::core::prelude::v1::derive`]: https://doc.rust-lang.org/core/prelude/v1/attr.derive.html
