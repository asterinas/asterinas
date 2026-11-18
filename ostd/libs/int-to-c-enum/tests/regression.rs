// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, TryFromInt)]
enum Color {
    Red = 1,
    Blue = 2,
    Green = 3,
}

#[test]
fn conversion() {
    let color = Color::try_from(1).unwrap();
    println!("color = {color:?}");
    assert!(color == Color::Red);
}

#[test]
fn invalid_value() {
    let color = Color::try_from(4);
    assert!(color.is_err());
}

#[test]
fn implicit_discriminants() {
    #[repr(u8)]
    #[derive(Debug, Eq, PartialEq, TryFromInt)]
    enum Color {
        Red,
        Yellow,
        Blue,
    }

    assert_eq!(Color::try_from(0).unwrap(), Color::Red);
    assert_eq!(Color::try_from(1).unwrap(), Color::Yellow);
    assert_eq!(Color::try_from(2).unwrap(), Color::Blue);
    assert!(Color::try_from(3).is_err());
}

#[test]
fn mixed_discriminants() {
    #[repr(i8)]
    #[derive(Debug, Eq, PartialEq, TryFromInt)]
    enum Value {
        Zero,
        One,
        Ten = 10,
        Eleven,
        NegativeThree = -3,
        NegativeTwo,
    }

    assert_eq!(Value::try_from(0).unwrap(), Value::Zero);
    assert_eq!(Value::try_from(1).unwrap(), Value::One);
    assert_eq!(Value::try_from(10).unwrap(), Value::Ten);
    assert_eq!(Value::try_from(11).unwrap(), Value::Eleven);
    assert_eq!(Value::try_from(-3).unwrap(), Value::NegativeThree);
    assert_eq!(Value::try_from(-2).unwrap(), Value::NegativeTwo);
    assert!(Value::try_from(2).is_err());
    assert!(Value::try_from(-1).is_err());
}

#[test]
fn constant_expression_discriminants() {
    #[repr(u8)]
    #[derive(Debug, Eq, PartialEq, TryFromInt)]
    enum Value {
        Eight = 1 << 3,
        Nine,
        Eleven = Self::Nine as u8 + 2,
        Twelve,
        AlmostMax = u8::MAX - 1,
        Max,
    }

    assert_eq!(Value::try_from(8).unwrap(), Value::Eight);
    assert_eq!(Value::try_from(9).unwrap(), Value::Nine);
    assert_eq!(Value::try_from(11).unwrap(), Value::Eleven);
    assert_eq!(Value::try_from(12).unwrap(), Value::Twelve);
    assert_eq!(Value::try_from(254).unwrap(), Value::AlmostMax);
    assert_eq!(Value::try_from(255).unwrap(), Value::Max);
    assert!(Value::try_from(10).is_err());
}
