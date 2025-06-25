// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

pub fn dump_profraw() {
    let mut coverage = Vec::new();
    unsafe {
        minicov::capture_coverage(&mut coverage).unwrap();
    }

    let coverage = coverage.leak();
    crate::early_println!("#### Coverage: {:p} {}", coverage.as_ptr(), coverage.len());
}
