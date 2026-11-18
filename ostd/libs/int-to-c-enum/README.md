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
int-to-c-enum = "0.2.0"
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

## Custom Errors

By default, failed conversions return [`TryFromIntError`].
To customize the error, specify `error_type` and exactly one of `error_fn` or `error_value`
in an enum-level `#[try_from_int(...)]` attribute.

```rust
use int_to_c_enum::TryFromInt;

/// An error converting an integer to a color.
#[derive(Debug, Eq, PartialEq)]
enum ColorError {
    InvalidValue,
    InvalidInteger(u8),
}

/// A supported color.
#[repr(u8)]
#[derive(Debug, Eq, PartialEq, TryFromInt)]
#[try_from_int(
    error_type = ColorError,
    error_value = ColorError::InvalidValue,
    // To include the invalid integer in the error, replace error_value with error_fn.
    // error_fn = ColorError::InvalidInteger,
)]
enum Color {
    Red = 1,
    Yellow,
    Blue,
}

assert_eq!(Color::try_from(1), Ok(Color::Red));
assert_eq!(Color::try_from(4), Err(ColorError::InvalidValue));
```

`error_fn` accepts a callable expression, such as a function path or closure.
It receives the original integer with the enum's `repr` type
and returns the specified `error_type`.
The expression is evaluated and called once per failed conversion.
Successful conversions do not evaluate it.

`error_value` accepts an expression that produces the specified `error_type`,
such as an enum variant, a struct constructor, or a function call.
It is evaluated once per failed conversion.
For example, `error_type = InvalidColor, error_value = InvalidColor`
returns a unit struct error without a closure.

## Macro Expansion

The `TryFromInt` macro will automatically implement trait `TryFrom<u8>` for `Color`.
After macro expansion, the generated code looks like as follows:

```ignore
impl TryFrom<u8> for Color {
    type Error = TryFromIntError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        {
            const __TRY_FROM_INT_DISCRIMINANT_0: u8 = Color::Red as u8;
            const __TRY_FROM_INT_DISCRIMINANT_1: u8 = Color::Yellow as u8;
            const __TRY_FROM_INT_DISCRIMINANT_2: u8 = Color::Blue as u8;

            match value {
                __TRY_FROM_INT_DISCRIMINANT_0 => return Ok(Color::Red),
                __TRY_FROM_INT_DISCRIMINANT_1 => return Ok(Color::Yellow),
                __TRY_FROM_INT_DISCRIMINANT_2 => return Ok(Color::Blue),
                _ => {}
            }
        }
        Err(TryFromIntError)
    }
}
```

## License

This project is licensed under MPL-2.0.
