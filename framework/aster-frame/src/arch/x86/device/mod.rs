// SPDX-License-Identifier: MPL-2.0

//! Device-related APIs.
//! This module mainly contains the APIs that should exposed to the device driver like PCI, RTC

// FIXME: remove this lint when the documentation of this module is extensively added.
#![allow(missing_docs)]

pub mod cmos;
pub mod io_port;
pub mod serial;
