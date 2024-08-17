// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicI8, AtomicU8};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

/// The process scheduling nice value.
///
/// It is an integer in the range of [-20, 19]. Process with a smaller nice
/// value is more favorable in scheduling.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Nice(NiceRange);

pub type NiceRange = RangedI8<-20, 19>;

define_atomic_version_of_integer_like_type!(Nice, try_from = true, {
    #[derive(Debug)]
    pub struct AtomicNice(AtomicI8);
});

impl Nice {
    pub const fn new(range: NiceRange) -> Self {
        Self(range)
    }

    pub const fn range(&self) -> &NiceRange {
        &self.0
    }

    pub fn range_mut(&mut self) -> &mut NiceRange {
        &mut self.0
    }
}

impl Default for Nice {
    fn default() -> Self {
        Self::new(NiceRange::new(0))
    }
}

impl From<Nice> for i8 {
    fn from(value: Nice) -> Self {
        value.0.into()
    }
}

impl TryFrom<i8> for Nice {
    type Error = <NiceRange as TryFrom<i8>>::Error;

    fn try_from(value: i8) -> Result<Self, Self::Error> {
        let range = value.try_into()?;
        Ok(Self::new(range))
    }
}

/// The thread scheduling priority value.
///
/// It is an integer in the range of [0, 139]. Here we follow the Linux
/// priority mappings: the relation between [`Priority`] and [`Nice`] is
/// as such - prio = nice + 120 while the priority of [0, 100] are
/// reserved for real-time tasks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Priority(PriorityRange);

pub type PriorityRange = RangedU8<0, 139>;

define_atomic_version_of_integer_like_type!(Priority, try_from = true, {
    #[derive(Debug)]
    pub struct AtomicPriority(AtomicU8);
});

impl Priority {
    pub const fn new(range: PriorityRange) -> Self {
        Self(range)
    }

    pub const fn default_real_time() -> Self {
        Self::new(PriorityRange::new(50))
    }

    pub const fn idle() -> Self {
        Self::new(PriorityRange::new(PriorityRange::MAX))
    }

    pub const fn range(&self) -> &PriorityRange {
        &self.0
    }

    pub fn range_mut(&mut self) -> &mut PriorityRange {
        &mut self.0
    }
}

impl From<Nice> for Priority {
    fn from(value: Nice) -> Self {
        Self::new(PriorityRange::new(value.range().get() as u8 + 120))
    }
}

impl From<Priority> for Nice {
    fn from(priority: Priority) -> Self {
        Self::new(NiceRange::new((priority.range().get() - 100) as i8 - 20))
    }
}

impl Default for Priority {
    fn default() -> Self {
        Nice::default().into()
    }
}

impl From<Priority> for u8 {
    fn from(value: Priority) -> Self {
        value.0.into()
    }
}

impl TryFrom<u8> for Priority {
    type Error = <PriorityRange as TryFrom<u8>>::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let range = value.try_into()?;
        Ok(Self::new(range))
    }
}

macro_rules! define_ranged_integer {
    ($visibility: vis, $name: ident, $type: ty) => {
        #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
        $visibility struct $name<const MIN: $type, const MAX: $type>($type);

        impl<const MIN: $type, const MAX: $type> $name<MIN, MAX> {
            $visibility const MIN: $type = MIN as $type;
            $visibility const MAX: $type = MAX as $type;

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
                if value < Self::MIN || value > Self::MAX {
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
