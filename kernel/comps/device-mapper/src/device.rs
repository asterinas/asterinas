// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;

use aster_block::{
    BlockDevice, BlockDeviceMeta,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use device_id::DeviceId;

use crate::table::DmTable;

/// A mapped block device backed by a device-mapper table.
#[derive(Debug)]
pub struct DmDevice {
    name: String,
    id: DeviceId,
    table: DmTable,
    queue: BioRequestSingleQueue,
}

impl DmDevice {
    pub(crate) fn new(name: String, id: DeviceId, table: DmTable) -> Self {
        Self {
            name,
            id,
            table,
            queue: BioRequestSingleQueue::new(),
        }
    }

    /// Dequeues one mapped-device request and processes it.
    pub fn handle_requests(&self) {
        let request = self.queue.dequeue();
        self.handle_request(request);
    }

    fn handle_request(&self, request: BioRequest) {
        for bio in request.into_bios() {
            self.handle_bio(bio);
        }
    }

    fn handle_bio(&self, bio: SubmittedBio) {
        if bio.type_() == BioType::Flush {
            let status = self
                .table
                .segments()
                .iter()
                .map(|segment| segment.target.flush())
                .find(|status| *status != BioStatus::Complete)
                .unwrap_or(BioStatus::Complete);
            bio.complete(status);
            return;
        }

        let start_sector = bio.sid_range().start.to_raw() + bio.sid_offset();
        let end_sector = bio.sid_range().end.to_raw() + bio.sid_offset();
        if end_sector > self.table.total_sectors() as u64 || start_sector >= end_sector {
            bio.complete(BioStatus::IoError);
            return;
        }

        let Some(segment) = self.table.find_segment(start_sector, end_sector) else {
            bio.complete(BioStatus::IoError);
            return;
        };
        let target_start_sector = start_sector - segment.start_sector;
        segment.target.handle_bio(bio, target_start_sector);
    }
}

impl BlockDevice for DmDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)?;
        Ok(())
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: self.table.total_sectors(),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }
}
