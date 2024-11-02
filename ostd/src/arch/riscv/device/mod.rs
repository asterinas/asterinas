// SPDX-License-Identifier: MPL-2.0

//! Device-related APIs.
//! This module mainly contains the APIs that should exposed to the device driver like RTC

pub mod goldfish_rtc;
pub mod io_port;

pub(crate) fn init() {
    goldfish_rtc::init();
}
