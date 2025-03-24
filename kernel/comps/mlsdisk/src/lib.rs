// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![feature(let_chains)]
#![feature(negative_impls)]
#![feature(slice_as_chunks)]
#![expect(dead_code, unused_imports)]

mod error;
mod layers;
mod os;
mod prelude;
mod tx;
mod util;

extern crate alloc;

use alloc::{string::ToString, sync::Arc, vec};
use core::ops::Range;

use aster_block::{
    bio::{Bio, BioDirection, BioSegment, BioStatus, BioType},
    id::Sid,
    BlockDevice, SECTOR_SIZE,
};
use component::{init_component, ComponentInitError};
use ostd::{mm::VmIo, prelude::*};

pub use self::{
    error::{Errno, Error},
    layers::{
        bio::{BlockId, BlockSet, Buf, BufMut, BufRef, BLOCK_SIZE},
        disk::MlsDisk,
    },
    os::{Aead, AeadIv, AeadKey, AeadMac, Rng},
    util::{Aead as _, RandomInit, Rng as _},
};

#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    // FIXME: add a virtio-blk-pci device in qemu and a image file.
    let Some(device) = aster_block::get_device("raw_mlsdisk") else {
        return Err(ComponentInitError::Unknown);
    };
    let raw_disk = RawDisk::new(device);
    let root_key = AeadKey::random();
    let device =
        MlsDisk::create(raw_disk, root_key, None).map_err(|_| ComponentInitError::Unknown)?;
    aster_block::register_device("mlsdisk".to_string(), Arc::new(device));
    Ok(())
}

#[derive(Clone, Debug)]
struct RawDisk {
    inner: Arc<dyn BlockDevice>,
    region: Range<BlockId>,
}

impl RawDisk {
    fn new(host_disk: Arc<dyn BlockDevice>) -> Self {
        let end = host_disk.metadata().nr_sectors * SECTOR_SIZE / BLOCK_SIZE;
        Self {
            inner: host_disk,
            region: Range { start: 0, end },
        }
    }
}

impl BlockSet for RawDisk {
    fn read(&self, pos: BlockId, mut buf: BufMut) -> core::result::Result<(), Error> {
        if pos + buf.nblocks() > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "read position is out of range");
        }
        let sid = Sid::from_offset((self.region.start + pos) * BLOCK_SIZE);
        let bio_segment = BioSegment::alloc(buf.nblocks(), BioDirection::FromDevice);
        let bio = Bio::new(BioType::Read, sid, vec![bio_segment.clone()], None);

        let res = bio.submit_and_wait(&*self.inner);
        match res {
            Ok(BioStatus::Complete) => {
                bio_segment.read_bytes(0, buf.as_mut_slice()).unwrap();
                Ok(())
            }
            _ => return_errno_with_msg!(Errno::IoFailed, "read io failed"),
        }
    }

    fn write(&self, pos: BlockId, buf: BufRef) -> core::result::Result<(), Error> {
        if pos + buf.nblocks() > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "write position is out of range");
        }
        let sid = Sid::from_offset((self.region.start + pos) * BLOCK_SIZE);
        let bio_segment = BioSegment::alloc(buf.nblocks(), BioDirection::ToDevice);
        bio_segment.write_bytes(0, buf.as_slice()).unwrap();
        let bio = Bio::new(BioType::Write, sid, vec![bio_segment], None);

        let res = bio.submit_and_wait(&*self.inner);
        match res {
            Ok(BioStatus::Complete) => Ok(()),
            _ => return_errno_with_msg!(Errno::IoFailed, "write io failed"),
        }
    }

    fn subset(&self, range: Range<BlockId>) -> core::result::Result<Self, Error> {
        if self.region.start + range.end > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "subset is out of range");
        }

        Ok(RawDisk {
            inner: self.inner.clone(),
            region: Range {
                start: self.region.start + range.start,
                end: self.region.start + range.end,
            },
        })
    }

    fn flush(&self) -> core::result::Result<(), Error> {
        Ok(())
    }

    fn nblocks(&self) -> usize {
        self.region.len()
    }
}

#[cfg(ktest)]
mod test {
    use aster_block::{
        bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
        BlockDevice, BlockDeviceMeta, SECTOR_SIZE,
    };
    use ostd::{
        mm::{FrameAllocOptions, Segment, VmIo},
        prelude::*,
    };

    use super::*;

    #[derive(Debug)]
    struct MemoryDisk {
        blocks: Segment<()>,
    }

    impl MemoryDisk {
        fn new(nblocks: usize) -> Self {
            let blocks = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(nblocks)
                .unwrap();
            Self { blocks }
        }
    }

    impl BlockDevice for MemoryDisk {
        fn enqueue(&self, bio: SubmittedBio) -> core::result::Result<(), BioEnqueueError> {
            let bio_type = bio.type_();
            if bio_type == BioType::Flush || bio_type == BioType::Discard {
                bio.complete(BioStatus::Complete);
                return Ok(());
            }

            let mut current_offset = bio.sid_range().start.to_offset();
            for segment in bio.segments() {
                let size = match bio_type {
                    BioType::Read => segment
                        .inner_segment()
                        .writer()
                        .write(self.blocks.reader().skip(current_offset)),
                    BioType::Write => self
                        .blocks
                        .writer()
                        .skip(current_offset)
                        .write(&mut segment.inner_segment().reader()),
                    _ => 0,
                };
                current_offset += size;
            }
            bio.complete(BioStatus::Complete);
            Ok(())
        }

        fn metadata(&self) -> BlockDeviceMeta {
            BlockDeviceMeta {
                max_nr_segments_per_bio: usize::MAX,
                nr_sectors: self.blocks.size() / SECTOR_SIZE,
            }
        }
    }

    fn create_rawdisk(nblocks: usize) -> RawDisk {
        let memory_disk = MemoryDisk::new(nblocks);
        RawDisk::new(Arc::new(memory_disk))
    }

    #[ktest]
    fn write_sync_read() {
        let nblocks = 64 * 1024;
        let raw_disk = create_rawdisk(nblocks);
        let root_key = AeadKey::random();
        let mlsdisk = MlsDisk::create(raw_disk.clone(), root_key, None).unwrap();

        let num_rw = 128;
        let mut rw_buf = Buf::alloc(1).unwrap();
        for i in 0..num_rw {
            rw_buf.as_mut_slice().fill(i as u8);
            mlsdisk.write(i, rw_buf.as_ref()).unwrap();
        }

        mlsdisk.sync().unwrap();

        for i in 0..num_rw {
            mlsdisk.read(i, rw_buf.as_mut()).unwrap();
            assert_eq!(rw_buf.as_slice()[0], i as u8);
        }
    }
}
