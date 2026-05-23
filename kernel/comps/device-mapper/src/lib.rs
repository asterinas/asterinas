// SPDX-License-Identifier: MPL-2.0

//! Device-mapper support for Asterinas.
//!
//! This component implements a small, table-driven virtual block-device layer
//! inspired by Linux device-mapper. A mapped device is described by a table of
//! sector ranges, each handled by a target that decides how the range is backed
//! by lower devices.
//!
//! Reference: Linux device-mapper documentation
//! <https://docs.kernel.org/admin-guide/device-mapper/>.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "dm: "
    };
}

mod device;
mod error;
mod table;
pub mod target;

use component::{ComponentInitError, init_component};

pub use self::{
    device::DmDevice,
    error::{DmError, DmErrorWithContext},
    table::{DmTable, DmTableSegment},
};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    Ok(())
}
