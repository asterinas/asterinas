// SPDX-License-Identifier: MPL-2.0

//! Support for the code coverage feature of OSDK.
//!
//! For more information about the code coverage feature (`cargo osdk run --coverage`),
//! check out the OSDK reference manual.

use alloc::vec::Vec;
use core::mem::ManuallyDrop;

/// A hook to be invoked on QEMU exit for dumping the code coverage data.
pub(crate) fn on_qemu_exit() {
    let mut coverage = ManuallyDrop::new(Vec::new());
    unsafe {
        minicov::capture_coverage(&mut *coverage).unwrap();
    }

    crate::early_println!("#### Coverage: {:p} {}", coverage.as_ptr(), coverage.len());
}
