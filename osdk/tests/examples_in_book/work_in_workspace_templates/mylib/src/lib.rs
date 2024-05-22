// SPDX-License-Identifier: MPL-2.0

pub fn available_memory() -> usize {
    let regions = aster_frame::boot::memory_regions();
    regions.iter().map(|region| region.len()).sum()
}
