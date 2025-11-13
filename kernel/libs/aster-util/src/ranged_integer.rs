// SPDX-License-Identifier: MPL-2.0

//! Ranged integer types.
//!
//! This module provides generic ranged integer types that enforce value always stay
//! within the specified range.

macro_rules! define_ranged_integer {
    ($visibility: vis, $name: ident, $type: ty) => {
        #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
        $visibility struct $name<const MIN: $type, const MAX: $type>($type);

        impl<const MIN: $type, const MAX: $type> $name<MIN, MAX> {
            $visibility const MIN: Self = Self::new(MIN);
            $visibility const MAX: Self = Self::new(MAX);

            $visibility const fn new(val: $type) -> Self {
                assert!(val >= MIN && val <= MAX);
                Self(val)
            }

            $visibility fn set(&mut self, val: $type) {
                assert!(val >= MIN && val <= MAX);
                self.0 = val;
            }

            $visibility const fn get(self) -> $type {
                self.0
            }
        }

        impl<const MIN: $type, const MAX: $type> From<$name<MIN, MAX>> for $type {
            fn from(value: $name<MIN, MAX>) -> Self {
                value.0
            }
        }

        impl<const MIN: $type, const MAX: $type> TryFrom<$type> for $name<MIN, MAX> {
            type Error = &'static str;

            fn try_from(value: $type) -> Result<Self, Self::Error> {
                if value < MIN || value > MAX {
                    Err("Initialized with out-of-range value.")
                } else {
                    Ok(Self(value))
                }
            }
        }
    };
}

define_ranged_integer!(pub, RangedI8, i8);
define_ranged_integer!(pub, RangedU8, u8);
define_ranged_integer!(pub, RangedI16, i16);
define_ranged_integer!(pub, RangedU16, u16);
define_ranged_integer!(pub, RangedI32, i32);
define_ranged_integer!(pub, RangedU32, u32);
