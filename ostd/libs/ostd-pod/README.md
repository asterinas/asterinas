# ostd-pod

A Rust library providing a marker trait and derive macros for Plain Old Data (POD) types.

## What is a POD Type?

A POD (Plain Old Data) type is a type that can be safely converted to and from an arbitrary byte sequence. For example, primitive types like `u8` and `i16` are POD types.

## Features

- **Safe Byte Conversion**: POD types can be safely converted to byte sequences and created from byte sequences
- **Based on zerocopy**: Built on top of the mature `zerocopy` crate for type safety guarantees
- **Derive Macro Support**: Provides `#[derive(Pod)]` macro to simplify POD type definitions
- **Union Support**: Supports union types via the `#[pod_union]` macro
- **Automatic Padding Management**: Automatically handles padding bytes through the `#[padding_struct]` macro

## Quick Start

Add the dependency to your `Cargo.toml`, zerocopy must be added as a dependency too.

```toml
[dependencies]
ostd-pod = "0.2.0"
zerocopy = { version = "0.8.34", features = ["derive" ] }
```

## Basic Usage

### Define a POD Struct

```rust
use ostd_pod::{derive, IntoBytes, Pod};

#[repr(C)]
#[derive(Pod, Clone, Copy, Debug)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let point = Point { x: 10, y: 20 };

    // Convert to bytes
    let bytes = point.as_bytes();
    println!("Bytes: {:?}", bytes);

    // Create from bytes
    let point2 = Point::from_bytes(bytes);
    println!("Point: {:?}", point2);
}
```

## License

This project is licensed under MPL-2.0.

## Related Links
- [Asterinas Project](https://github.com/asterinas/asterinas)
- [zerocopy crate](https://docs.rs/zerocopy/)
