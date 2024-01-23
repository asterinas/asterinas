// SPDX-License-Identifier: MPL-2.0

use bytemuck_derive::NoUninit;

use crate::prelude::*;

/// The process scheduling nice value.
///
/// The nice value is an attribute that can be used to influence the
/// CPU scheduler to favor or disfavor a process in scheduling decisions.
///
/// It is a value in the range -20 to 19, with -20 being the highest priority
/// and 19 being the lowest priority. The smaller values give a process a higher
/// scheduling priority.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, NoUninit)]
pub struct Nice {
    value: i8,
}

impl Nice {
    /// The minimum nice, whose value is -20.
    pub const MIN: Self = Self { value: -20 };

    /// The maximum nice, whose value is 19.
    pub const MAX: Self = Self { value: 19 };

    /// Creates a new `Nice` from the raw value.
    ///
    /// Values given beyond the permissible range are automatically adjusted
    /// to the nearest boundary value.
    pub fn new(raw: i8) -> Self {
        if raw > Self::MAX.to_raw() {
            Self::MAX
        } else if raw < Self::MIN.to_raw() {
            Self::MIN
        } else {
            Self { value: raw }
        }
    }

    /// Converts to the raw value.
    pub fn to_raw(self) -> i8 {
        self.value
    }
}

#[allow(clippy::derivable_impls)]
impl Default for Nice {
    fn default() -> Self {
        Self {
            // The default nice value is 0
            value: 0,
        }
    }
}

impl From<Priority> for Nice {
    fn from(priority: Priority) -> Self {
        Self {
            value: 20 - priority.to_raw() as i8,
        }
    }
}

/// The process scheduling priority value.
///
/// It is a value in the range 1 (corresponding to a nice value of 19)
/// to 40 (corresponding to a nice value of -20), with 1 being the lowest priority
/// and 40 being the highest priority. The greater values give a process a higher
/// scheduling priority.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, NoUninit)]
pub struct Priority {
    value: u8,
}

impl Priority {
    /// The minimum priority, whose value is 1.
    pub const MIN: Self = Self { value: 1 };

    /// The maximum priority, whose value is 40.
    pub const MAX: Self = Self { value: 40 };

    /// Creates a new `Priority` from the raw value.
    ///
    /// Values given beyond the permissible range are automatically adjusted
    /// to the nearest boundary value.
    pub fn new(raw: u8) -> Self {
        if raw > Self::MAX.to_raw() {
            Self::MAX
        } else if raw < Self::MIN.to_raw() {
            Self::MIN
        } else {
            Self { value: raw }
        }
    }

    /// Converts to the raw value.
    pub fn to_raw(self) -> u8 {
        self.value
    }
}

impl From<Nice> for Priority {
    fn from(nice: Nice) -> Self {
        Self {
            value: (20 - nice.to_raw()) as u8,
        }
    }
}
