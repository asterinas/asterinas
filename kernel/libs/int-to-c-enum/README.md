# TryFromInt - A convenient derive macro for converting an integer to an enum

## Quick Start
To use this crate, first add this crate to your `Cargo.toml`.

```toml
[dependencies]
int-to-c-enum = "0.1.0"
```

You can use this macro for a [C-like enum](https://doc.rust-lang.org/stable/rust-by-example/custom_types/enum/c_like.html).

```rust 
use int_to_c_enum::TryFromInt;
#[repr(u8)]
#[derive(TryFromInt, Debug)]
pub enum Color {
    Red = 1,
    Blue = 2,
    Green = 3,
}
```

Then, you can use `try_from` function for this enum.
```rust
fn main() {
    let color = Color::try_from(1).unwrap();
    println!("color = {color:?}"); // color = Red;
}
```

## Introduction
This crate provides a derive procedural macro named `TryFromInt`. This macro will automatically implement [TryFrom](https://doc.rust-lang.org/core/convert/trait.TryFrom.html) trait for enums that meet the following requirements:
1. The enum must have a primitive repr, i.e., the enum should have attribute like #[repr(u8)], #[repr(u32)], etc. The type parameter of TryFrom will be the repr, e.g., in the `QuickStart` example, the macro will implement `TryFrom<u8>` for `Color`.
2. The enum must consist solely of unit variants, which is called [units only enum](https://doc.rust-lang.org/reference/items/enumerations.html#unit-only-enum). Each field should have an **explicit discriminant**.
