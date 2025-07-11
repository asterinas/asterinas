// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

pub fn dump_profraw() -> Vec<u8> {
    let mut coverage = Vec::new();
    unsafe {
        minicov::capture_coverage(&mut coverage).unwrap();
    }

    coverage
}
