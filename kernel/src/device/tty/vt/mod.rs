// SPDX-License-Identifier: MPL-2.0

mod console;
mod driver;
mod file;
mod ioctl_defs;
mod keyboard;
mod manager;

use core::num::NonZeroU8;

pub use driver::VtDriver;
pub use manager::active_vt;

use crate::prelude::Result;

const MAX_CONSOLES: usize = 63;

/// Virtual terminal index and always in the range `1..=MAX_CONSOLES`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct VtIndex(NonZeroU8);

impl VtIndex {
    /// Creates a `VtIndex` from a 1-based VT number.
    ///
    /// Returns `None` if `value == 0` or `value > MAX_CONSOLES`.
    const fn new(value: u8) -> Option<Self> {
        if value == 0 || value as usize > MAX_CONSOLES {
            None
        } else {
            Some(VtIndex(NonZeroU8::new(value).unwrap()))
        }
    }

    /// Returns the 1-based VT number.
    #[inline]
    const fn get(self) -> u8 {
        self.0.get()
    }

    /// Converts this VT number to a 0-based index for internal array access.
    ///
    /// Safe because `VtIndex` is guaranteed to be non-zero.
    #[inline]
    const fn to_zero_based(self) -> usize {
        (self.get() as usize) - 1
    }

    /// Returns the next VT index, wrapping around to VT1 after reaching VT`MAX_CONSOLES`.
    fn next_wrap(self) -> Self {
        let next = if (self.get() as usize) >= MAX_CONSOLES {
            1
        } else {
            self.get() + 1
        };

        Self(NonZeroU8::new(next).unwrap())
    }

    /// Returns the previous VT index, wrapping around to VT`MAX_CONSOLES` when at VT1.
    fn prev_wrap(self) -> Self {
        let prev = if self.get() == 1 {
            MAX_CONSOLES as u8
        } else {
            self.get() - 1
        };

        Self(NonZeroU8::new(prev).unwrap())
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    keyboard::init();
    manager::init()?;
    Ok(())
}
