// SPDX-License-Identifier: MPL-2.0

//! The `linear` target.
//!
//! Maps a contiguous range of the mapped device onto a contiguous range of a
//! lower device, shifting every sector by a fixed offset. This is the simplest
//! device-mapper target and the building block for partition-like remapping.
//!
//! Reference: Linux `Documentation/admin-guide/device-mapper/linear.rst`.

use alloc::sync::Arc;

use aster_block::{
    BlockDevice,
    bio::{Bio, BioStatus, SubmittedBio},
    id::Sid,
};

use super::DmTarget;

/// A `linear` target backed by a lower block device.
#[derive(Debug)]
pub struct LinearTarget {
    lower: Arc<dyn BlockDevice>,
    /// The sector on the lower device that the segment's first sector maps to.
    start_sector: u64,
}

impl LinearTarget {
    /// Creates a `linear` target that forwards I/O to `lower`, shifting sectors
    /// so that the segment's first sector lands on `start_sector`.
    pub fn new(lower: Arc<dyn BlockDevice>, start_sector: u64) -> Self {
        Self {
            lower,
            start_sector,
        }
    }
}

impl DmTarget for LinearTarget {
    fn type_name(&self) -> &'static str {
        "linear"
    }

    fn handle_bio(&self, bio: SubmittedBio, target_start_sector: u64) {
        // Reissue the request to the lower device with the same memory segments
        // (which share the underlying DMA buffers) at the remapped sector.
        let request_sectors = bio.sid_range().end.to_raw() - bio.sid_range().start.to_raw();
        let Some(lower_start_sector) = self.start_sector.checked_add(target_start_sector) else {
            bio.complete(BioStatus::IoError);
            return;
        };
        let Some(lower_end_sector) = lower_start_sector.checked_add(request_sectors) else {
            bio.complete(BioStatus::IoError);
            return;
        };
        if lower_end_sector > self.lower.metadata().nr_sectors as u64 {
            bio.complete(BioStatus::IoError);
            return;
        }

        let lower_start = Sid::new(lower_start_sector);
        let lower_bio = Bio::new(bio.type_(), lower_start, bio.segments().to_vec(), None);
        let status = lower_bio
            .submit_and_wait(self.lower.as_ref())
            .unwrap_or(BioStatus::IoError);
        bio.complete(status);
    }

    fn flush(&self) -> BioStatus {
        self.lower.sync().unwrap_or(BioStatus::IoError)
    }
}
