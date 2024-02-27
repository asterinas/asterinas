// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

#[derive(TryFromInt, Debug, PartialEq, Eq)]
#[repr(u8)]
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
