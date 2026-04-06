// SPDX-License-Identifier: MPL-2.0

//! Virtual terminal (VT) subsystem.
//!
//! This module implements Linux-compatible VT behavior for `/dev/ttyN` (`N >= 1`) and
//! exposes the currently active VT through `/dev/tty0` (via the active-console device).
//!
//! A VT is identified by a 1-based index ([`VtIndex`]). The subsystem manages multiple
//! VTs and presents one of them as the active terminal at a time.
//!
//! At boot, VT1 starts as the active VT and is allocated by default. Other VTs are
//! allocated on demand when user-visible operations require them, such as opening
//! `/dev/ttyN` or activating a VT with the `VT_ACTIVATE` ioctl.
//!
//! The `VT_DISALLOCATE` ioctl requests VT deallocation. This request is rejected if the
//! target VT is busy, including the case where user space still holds open file handles
//! for that VT. Open-file reference tracking exists so this user-visible rule can be
//! enforced correctly: deallocation must fail while handles are open and may succeed only
//! after those handles are closed.
//!
//! VT switching is visible through keyboard VT switching and the `VT_ACTIVATE` ioctl.
//! The `VT_WAITACTIVE` ioctl waits until a given VT becomes active. VT switching policy
//! is configured with the `VT_SETMODE` ioctl ([`manager::VtMode`]): in `Auto` mode, the
//! kernel completes switches directly; in `Process` mode, switching away is coordinated
//! with user space via the `VT_RELDISP` ioctl.
//!
//! Console display mode is configured with the `KDSETMODE` ioctl:
//! - In text mode, the VT behaves as a text terminal.
//! - In graphics mode, the VT is treated as graphics-owned display space.
//!
//! This distinction affects switching behavior. Some kernel-controlled switch paths are
//! rejected when the active VT is in graphics mode.

mod c_types;
mod device;
mod driver;
mod file;
mod ioctl_defs;
mod keyboard;
mod manager;

use core::num::NonZeroU8;

pub(super) use driver::VtDriver;
pub(super) use manager::active_vt;

use crate::{
    error::return_errno_with_message,
    prelude::{Errno, Result},
};

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

    /// Returns the 1-based VT number.
    const fn get(self) -> u8 {
        self.0.get()
    }

    /// Converts this VT number to a 0-based index for array access.
    ///
    /// Infallible because `VtIndex` is guaranteed to be non-zero.
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

impl TryFrom<i32> for VtIndex {
    type Error = crate::prelude::Error;

    fn try_from(value: i32) -> Result<Self> {
        if let Ok(value) = u8::try_from(value)
            && let Some(index) = Self::new(value)
        {
            Ok(index)
        } else {
            return_errno_with_message!(Errno::ENXIO, "the VT index is out of range");
        }
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    keyboard::init_in_first_process();
    manager::init_in_first_process()?;
    device::init_in_first_process();
    Ok(())
}
