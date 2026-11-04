// SPDX-License-Identifier: MPL-2.0

//! Intel TDX attestation interfaces.

use alloc::boxed::Box;

use crate::prelude::Result;

/// Returns whether the kernel is running as an Intel TDX guest.
pub fn is_enabled() -> bool {
    tdx_guest::tdx_is_enabled()
}

/// Generates a TDX quote for the supplied report data.
pub fn get_quote(report_data: &[u8]) -> Result<Box<[u8]>> {
    crate::device::misc::tdxguest::tdx_get_quote(report_data)
}
