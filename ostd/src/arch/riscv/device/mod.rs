// SPDX-License-Identifier: MPL-2.0

//! Device-related APIs.
//! This module mainly contains the APIs that should exposed to the device driver like PCI, RTC

pub mod io_port;
pub mod plic;

pub(crate) fn init() {
    plic::init();
}
