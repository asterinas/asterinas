<!--
To promote a "single source of truth", the content of `README.md` is also included in `lib.rs`
as the crate-level documentation. So when writing this README, bear in mind that its content
should be recognized correctly by both a Markdown renderer and the rustdoc tool.
-->

# TryFromInt

A convenient derive macro for converting an integer to a
[C-like enum](https://doc.rust-lang.org/stable/rust-by-example/custom_types/enum/c_like.html).

This crate provides a derive procedural macro named `TryFromInt`.
This macro will automatically implement
[`TryFrom`](https://doc.rust-lang.org/core/convert/trait.TryFrom.html) trait
for enums that meet the following requirements:

1. The enum must have a primitive repr, i.e., the enum should have attribute like
   `#[repr(u8)]`, `#[repr(u32)]`, etc. The type parameter of `TryFrom` will be
   the repr.
2. The enum must consist solely of unit variants, which is called
   [units only enum](https://doc.rust-lang.org/reference/items/enumerations.html#unit-only-enum).

## Quick Start

To use this crate, first add this crate to your `Cargo.toml`.

```toml
[dependencies]
int-to-c-enum = "0.1.0"
```

Below is a simple example. We derive macro `TryFromInt` for an enum `Color`.

```rust
use int_to_c_enum::TryFromInt;

#[repr(u8)]
#[derive(TryFromInt, Debug, Eq, PartialEq)]
pub enum Color {
    Red = 1, // Support explicit discriminants...
    Yellow,  // and implicit ones as well.
    Blue,
}

// Then, we can use method `try_from` for `Color`.
let color = Color::try_from(1).unwrap();
assert!(color == Color::Red);
```

## Macro Expansion

The `TryFromInt` macro will automatically implement trait `TryFrom<u8>` for `Color`.
After macro expansion, the generated code looks like as follows:

```ignore
impl TryFrom<u8> for Color {
    type Error = TryFromIntError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        const __TRY_FROM_INT_DISCRIMINANT_0: u8 = Color::Red as u8;
        const __TRY_FROM_INT_DISCRIMINANT_1: u8 = Color::Yellow as u8;
        const __TRY_FROM_INT_DISCRIMINANT_2: u8 = Color::Blue as u8;

        match value {
            __TRY_FROM_INT_DISCRIMINANT_0 => Ok(Color::Red),
            __TRY_FROM_INT_DISCRIMINANT_1 => Ok(Color::Yellow),
            __TRY_FROM_INT_DISCRIMINANT_2 => Ok(Color::Blue),
            _ => Err(TryFromIntError),
        }
    }
}
```

## License

This project is licensed under MPL-2.0.
