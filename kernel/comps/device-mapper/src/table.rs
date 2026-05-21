// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};

use crate::{DmError, target::DmTarget};

/// One mapped interval in a device-mapper table.
#[derive(Debug)]
pub struct DmTableSegment {
    pub start_sector: u64,
    pub len_sectors: u64,
    pub target: Box<dyn DmTarget>,
}

impl DmTableSegment {
    pub fn end_sector(&self) -> Option<u64> {
        self.start_sector.checked_add(self.len_sectors)
    }

    pub fn contains_range(&self, start_sector: u64, end_sector: u64) -> bool {
        let Some(segment_end) = self.end_sector() else {
            return false;
        };

        self.start_sector <= start_sector && end_sector <= segment_end
    }
}

/// A device-mapper table.
///
/// BIOs that cross target boundaries are rejected; callers must split them
/// before dispatch.
#[derive(Debug)]
pub struct DmTable {
    segments: Vec<DmTableSegment>,
    total_sectors: u64,
}

impl DmTable {
    pub fn new(segments: Vec<DmTableSegment>) -> Result<Self, DmError> {
        if segments.is_empty() {
            return Err(DmError::InvalidTable);
        }

        let mut previous_end = 0;
        let mut total_sectors = 0;
        for segment in &segments {
            if segment.len_sectors == 0 {
                return Err(DmError::InvalidTable);
            }

            let Some(end_sector) = segment.end_sector() else {
                return Err(DmError::InvalidTable);
            };
            if segment.start_sector < previous_end {
                return Err(DmError::InvalidTable);
            }
            previous_end = end_sector;
            total_sectors = total_sectors.max(end_sector);
        }

        Ok(Self {
            segments,
            total_sectors,
        })
    }

    pub fn total_sectors(&self) -> u64 {
        self.total_sectors
    }

    pub fn segments(&self) -> &[DmTableSegment] {
        &self.segments
    }

    pub fn find_segment(&self, start_sector: u64, end_sector: u64) -> Option<&DmTableSegment> {
        self.segments
            .iter()
            .find(|segment| segment.contains_range(start_sector, end_sector))
    }
}
