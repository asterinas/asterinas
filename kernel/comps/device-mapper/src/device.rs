// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;

use aster_block::{
    BlockDevice, BlockDeviceMeta,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
};
use device_id::DeviceId;

use crate::table::DmTable;

/// A mapped block device backed by a device-mapper table.
#[derive(Debug)]
pub struct DmDevice {
    name: String,
    id: DeviceId,
    table: DmTable,
}

impl DmDevice {
    pub fn new(name: String, id: DeviceId, table: DmTable) -> Self {
        Self { name, id, table }
    }

    pub fn table(&self) -> &DmTable {
        &self.table
    }
}

impl BlockDevice for DmDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        if bio.type_() == BioType::Flush {
            let status = self
                .table
                .segments()
                .iter()
                .map(|segment| segment.target.flush())
                .find(|status| *status != BioStatus::Complete)
                .unwrap_or(BioStatus::Complete);
            bio.complete(status);
            return Ok(());
        }

        let start_sector = bio.sid_range().start.to_raw();
        let end_sector = bio.sid_range().end.to_raw();
        if end_sector > self.table.total_sectors() || start_sector >= end_sector {
            bio.complete(BioStatus::IoError);
            return Ok(());
        }

        let Some(segment) = self.table.find_segment(start_sector, end_sector) else {
            bio.complete(BioStatus::IoError);
            return Ok(());
        };
        let target_start_sector = start_sector - segment.start_sector;
        segment.target.handle_bio(bio, target_start_sector);
        Ok(())
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: usize::MAX,
            nr_sectors: self.table.total_sectors() as usize,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }
}
