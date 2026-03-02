<!--
To promote a "single source of truth", the content of `README.md` is also included in `lib.rs`
as the crate-level documentation. So when writing this README, bear in mind that its content
should be recognized correctly by both a Markdown renderer and the rustdoc tool.
-->

# ostd-pod

A trait and macros for Plain Old Data (POD) types.

This crate provides the [`Pod`] trait, 
which marks types that can be safely converted to and from arbitrary byte sequences. 
It's built on top of the mature [zerocopy] crate to ensure type safety.

## Features

- **Safe Byte Conversion**: POD types can be safely converted to byte sequences and created from
  byte sequences.
- **Based on zerocopy**: Built on top of the [zerocopy] crate for type safety guarantees.
- **Derive Macro Support**: Provides `#[derive(Pod)]` to simplify POD type definitions.
- **Union Support**: Supports union types via the `#[pod_union]` macro.
- **Automatic Padding Management**: Automatically handles padding bytes through the
  `#[padding_struct]` macro.

## What is a POD Type?

A POD (Plain Old Data) type is a type 
that can be safely converted to and from an arbitrary byte sequence. 
For example, primitive types like `u8` and `i16` are POD types; 
yet, `bool` is not a POD type. 
A struct whose fields are POD types is also considered a POD. 
A union whose fields are all POD types is also a POD. 
The memory layout of any POD type is `#[repr(C)]`.

## Quick Start

### Step 1: Edit your `Cargo.toml`

Add these dependencies to your `Cargo.toml`. 

```toml
[dependencies]
ostd-pod = "0.4.0"
zerocopy = { version = "0.8.34", features = ["derive" ] }
```

`zerocopy` must be explicitly specified as a dependency
because `ostd-pod` relies on its procedural macros, 
which expand to compile-time checks that reference internal `zerocopy`
types hardcoded to the `zerocopy` crate name.

### Step 2: Edit your `lib.rs` (or `main.rs`)

Insert the following lines to your `lib.rs` or `main.rs`:

```rust
#[macro_use]
extern crate ostd_pod;
```

We import the `ostd_pod` crate with `extern` and `#[macro_use]`
for the convenience of having Rust's built-in `derive` attribute macro
globally overridden by the custom `derive` attribute macro provided by this crate.
This custom `derive` macro is needed
because the `Pod` trait cannot be derived in the regular way as other traits.

### Step 3: Define your first POD type

Now we can define a POD struct that
can be converted to and from any byte sequence of the same size.

```rust
#[macro_use]
extern crate ostd_pod;
use ostd_pod::{IntoBytes, FromBytes, Pod};

#[repr(C)]
#[derive(Pod, Clone, Copy, Debug, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let point = Point { x: 10, y: 20 };

    // Convert to bytes
    let bytes = point.as_bytes();
    assert_eq!(bytes, &[10, 0, 0, 0, 20, 0, 0, 0]);

    // Create from bytes
    let point2 = Point::from_bytes(bytes);
    assert_eq!(point, point2);
}
```

## Advanced Usage

### Use POD Unions

Union fields cannot be accessed safely because we cannot know which variant is currently active.
To address this, we provide a [`pod_union`] macro
that enables safe access to union fields.

```rust
#[macro_use]
extern crate ostd_pod;
use ostd_pod::{FromZeros, IntoBytes};

#[pod_union]
#[derive(Copy, Clone)]
#[repr(C)]
union Data {
    value: u64,
    bytes: [u8; 4],
}

fn main() {
    let mut data = Data::new_value(0x1234567890ABCDEF);

    // Access the same memory through different fields
    assert_eq!(*data.value(), 0x1234567890ABCDEF);
    assert_eq!(*data.bytes(), [0xEF, 0xCD, 0xAB, 0x90]);
}
```

### Automatic Padding Handling

When a struct has fields with different sizes, 
there may be implicit padding bytes between fields.
The [`padding_struct`] macro automatically inserts explicit padding fields
so the struct can be safely used as a POD type.

```rust
#[macro_use]
extern crate ostd_pod;
use ostd_pod::IntoBytes;

#[repr(C)]
#[padding_struct]
#[derive(Pod, Clone, Copy, Debug, Default)]
struct PackedData {
    a: u8,
    // `padding_struct` automatically inserts 3 bytes of padding here
    b: u32,
    c: u16,
    // `padding_struct` automatically inserts 2 bytes of padding here
}

fn main() {
    let data = PackedData {
        a: 1,
        b: 2,
        c: 3,
        ..Default::default()
    };

    // Can safely convert to bytes, padding bytes are explicitly handled
    let bytes = data.as_bytes();
    assert_eq!(bytes.len(), 12);
    assert_eq!(bytes, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
}
```

## License

This project is licensed under MPL-2.0.

<!--
External links.
-->
[`Pod`]: https://docs.rs/ostd-pod/0.4.0/trait.Pod.html
[`padding_struct`]: https://docs.rs/ostd-pod/0.4.0/attr.padding_struct.html
[`pod_union`]: https://docs.rs/ostd-pod/0.4.0/attr.pod_union.html
[zerocopy]: https://docs.rs/zerocopy/