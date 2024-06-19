// SPDX-License-Identifier: MPL-2.0

pub fn available_memory() -> usize {
    let regions = ostd::boot::memory_regions();
    regions.iter().map(|region| region.len()).sum()
}
