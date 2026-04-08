// SPDX-License-Identifier: MPL-2.0

mod driver;
mod ioctl_defs;
mod keyboard;

use core::num::NonZeroU8;

pub(super) use driver::{VtDriver, tty1_device};

use crate::prelude::Result;

const MAX_CONSOLES: usize = 63;

/// A virtual terminal index that is always in the range `1..=MAX_CONSOLES`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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
}

pub(super) fn init_in_first_process() -> Result<()> {
    keyboard::init_in_first_process();
    driver::init_in_first_process()?;
    Ok(())
}
