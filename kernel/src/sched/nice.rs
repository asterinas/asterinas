// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicI8;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

/// The process scheduling nice value.
///
/// It is an integer in the range of [-20, 19]. Process with a smaller nice
/// value is more favorable in scheduling.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Nice(NiceValue);

pub type NiceValue = RangedI8<-20, 19>;

define_atomic_version_of_integer_like_type!(Nice, try_from = true, {
    #[derive(Debug)]
    pub struct AtomicNice(AtomicI8);
});

impl Nice {
    pub const MIN: Self = Nice::new(NiceValue::MIN);
    pub const MAX: Self = Nice::new(NiceValue::MAX);

    pub const fn new(range: NiceValue) -> Self {
        Self(range)
    }

    pub const fn value(&self) -> &NiceValue {
        &self.0
    }

    pub fn value_mut(&mut self) -> &mut NiceValue {
        &mut self.0
    }
}

impl Default for Nice {
    fn default() -> Self {
        Self::new(NiceValue::new(0))
    }
}

impl From<Nice> for i8 {
    fn from(value: Nice) -> Self {
        value.0.into()
    }
}

impl TryFrom<i8> for Nice {
    type Error = <NiceValue as TryFrom<i8>>::Error;

    fn try_from(value: i8) -> Result<Self, Self::Error> {
        let range = value.try_into()?;
        Ok(Self::new(range))
    }
}

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
