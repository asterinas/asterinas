// SPDX-License-Identifier: MPL-2.0

//! The assembler crate for the Asterinas kernel.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
extern crate alloc;

// High-level components as external crates.
//
// Keep this list in sync with the high-level component dependencies in `Cargo.toml`.
// Each high-level component crate must have an explicit `extern crate` declaration
// to ensure that its component registration and initialization code are linked into
// the kernel, because the assembler does not otherwise reference symbols from that crate.
extern crate aster_configfs as _;
extern crate aster_drm as _;
#[cfg(target_arch = "x86_64")]
extern crate aster_i8042 as _;
extern crate aster_simpledrm as _;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
extern crate aster_tsm_configfs;
extern crate aster_uart as _;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tdx {
    use alloc::boxed::Box;

    use aster_tsm_configfs::{ReportProvider, ReportProviderError};

    #[derive(Debug)]
    struct TdxReportProvider;

    impl ReportProvider for TdxReportProvider {
        fn name(&self) -> &'static str {
            "tdx_guest"
        }

        fn get_report(&self, inblob: &[u8]) -> Result<Box<[u8]>, ReportProviderError> {
            aster_core::security::tdx::get_quote(inblob).map_err(|_| ReportProviderError)
        }
    }

    static TDX_REPORT_PROVIDER: TdxReportProvider = TdxReportProvider;

    pub(super) fn register_report_provider() {
        if aster_core::security::tdx::is_enabled() {
            aster_tsm_configfs::register_report_provider(&TDX_REPORT_PROVIDER).unwrap();
        }
    }
}

#[ostd::main]
fn main() {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    tdx::register_report_provider();

    aster_core::boot();
}
