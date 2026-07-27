// SPDX-License-Identifier: MPL-2.0

//! Device-mapper support for Asterinas.
//!
//! This component implements a table-driven virtual block-device layer inspired
//! by Linux device-mapper. It supports `linear`, `zero`, and `error` targets
//! created from `dm-mod.create=`/`dm_mod.create=` kernel command-line entries.
//!
//! References:
//! - Linux device-mapper documentation:
//!   <https://docs.kernel.org/admin-guide/device-mapper/>

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "dm: "
    };
}

mod device;
mod error;
mod parser;
mod registry;
mod table;
pub mod target;

use alloc::vec::Vec;

use aster_block::BlockDevice;
use component::{ComponentInitError, init_component};
use spin::Once;

pub use self::{
    device::DmDevice,
    error::{DmError, DmErrorWithContext},
    parser::DmCreateArg,
};

static DM_CREATE_ARGS: Once<Vec<DmCreateArg>> = Once::new();
aster_cmdline::define_repeatable_kv_param!("dm_mod.create", DM_CREATE_ARGS);

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    registry::init().map_err(|_| ComponentInitError::Unknown)?;
    Ok(())
}

#[init_component(process)]
fn init_in_first_process() -> Result<(), ComponentInitError> {
    let create_args = DM_CREATE_ARGS.get().cloned().unwrap_or_default();
    for (index, arg) in create_args.iter().enumerate() {
        match parser::parse_create_arg(arg.as_str(), index) {
            Ok(parsed) => match registry::create_device(parsed.name.clone(), parsed.table) {
                Ok(device) => {
                    ostd::info!("created dm device '{}' ({:?})", device.name(), device.id());
                }
                Err(err) => {
                    ostd::error!("failed to create dm device '{}': {:?}", parsed.name, err);
                }
            },
            Err(err) => {
                ostd::error!(
                    "failed to parse dm-mod.create entry '{}': {:?}",
                    arg.as_str(),
                    err
                );
            }
        }
    }

    Ok(())
}

#[cfg(ktest)]
mod tests {
    use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
    use core::sync::atomic::{AtomicUsize, Ordering};

    use aster_block::{
        BLOCK_SIZE, BlockDevice, BlockDeviceMeta, SECTOR_SIZE,
        bio::{Bio, BioDirection, BioEnqueueError, BioSegment, BioStatus, BioType, SubmittedBio},
        id::Sid,
    };
    use device_id::{DeviceId, MajorId, MinorId};
    use io_util::batch::IoBatch;
    use ostd::{
        mm::{FrameAllocOptions, Segment, VmIo, io::util::HasVmReaderWriter},
        prelude::*,
    };

    use super::{
        device::DmDevice,
        table::{DmTable, DmTableSegment},
        target::{error::ErrorTarget, linear::LinearTarget, zero::ZeroTarget},
    };

    #[derive(Debug)]
    struct MemoryDisk {
        name: &'static str,
        id: DeviceId,
        bytes: Segment<()>,
        flushes: AtomicUsize,
    }

    #[derive(Debug)]
    struct OffsetBlockDevice {
        name: &'static str,
        id: DeviceId,
        inner: Arc<DmDevice>,
        sid_offset: u64,
    }

    impl MemoryDisk {
        fn new(name: &'static str, minor: u32, nblocks: usize) -> Self {
            let bytes = FrameAllocOptions::new()
                .zeroed(true)
                .alloc_segment(nblocks)
                .unwrap();
            Self {
                name,
                id: DeviceId::new(MajorId::new(250), MinorId::new(minor)),
                bytes,
                flushes: AtomicUsize::new(0),
            }
        }

        fn read_bytes_at(&self, offset: usize, out: &mut [u8]) {
            self.bytes.read_bytes(offset, out).unwrap();
        }

        fn write_bytes_at(&self, offset: usize, input: &[u8]) {
            self.bytes.write_bytes(offset, input).unwrap();
        }
    }

    impl BlockDevice for MemoryDisk {
        fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
            if bio.type_() == BioType::Flush {
                self.flushes.fetch_add(1, Ordering::Relaxed);
                bio.complete(BioStatus::Complete);
                return Ok(());
            }

            let mut offset = bio.sid_range().start.to_offset();
            for segment in bio.segments() {
                let Some(end_offset) = offset.checked_add(segment.nbytes()) else {
                    bio.complete(BioStatus::IoError);
                    return Ok(());
                };
                if end_offset > self.bytes.size() {
                    bio.complete(BioStatus::IoError);
                    return Ok(());
                }
                match bio.type_() {
                    BioType::Read => {
                        let _ = segment
                            .inner_dma_slice()
                            .writer()
                            .unwrap()
                            .write(self.bytes.reader().skip(offset));
                    }
                    BioType::Write => {
                        let _ = self
                            .bytes
                            .writer()
                            .skip(offset)
                            .write(&mut segment.inner_dma_slice().reader().unwrap());
                    }
                    BioType::Flush => unreachable!(),
                }
                offset = end_offset;
            }
            bio.complete(BioStatus::Complete);
            Ok(())
        }

        fn metadata(&self) -> BlockDeviceMeta {
            BlockDeviceMeta {
                max_nr_segments_per_bio: usize::MAX,
                nr_sectors: self.bytes.size() / SECTOR_SIZE,
            }
        }

        fn name(&self) -> &str {
            self.name
        }

        fn id(&self) -> DeviceId {
            self.id
        }
    }

    impl BlockDevice for OffsetBlockDevice {
        fn enqueue(&self, mut bio: SubmittedBio) -> Result<(), BioEnqueueError> {
            bio.set_sid_offset(self.sid_offset);
            self.inner.enqueue(bio)
        }

        fn metadata(&self) -> BlockDeviceMeta {
            let mut metadata = self.inner.metadata();
            metadata.nr_sectors = metadata.nr_sectors.saturating_sub(self.sid_offset as usize);
            metadata
        }

        fn name(&self) -> &str {
            self.name
        }

        fn id(&self) -> DeviceId {
            self.id
        }
    }

    fn read_offset_device(
        device: &OffsetBlockDevice,
        dm: &DmDevice,
        offset: usize,
        len: usize,
    ) -> (BioStatus, Vec<u8>) {
        let status = Arc::new(AtomicUsize::new(BioStatus::Init as usize));
        let complete_status = status.clone();
        let segment = BioSegment::alloc(len.div_ceil(BLOCK_SIZE), BioDirection::FromDevice);
        let bio = Bio::new(
            BioType::Read,
            Sid::from_offset(offset),
            vec![segment.clone()],
            Some(Box::new(move |bio_status| {
                complete_status.store(bio_status as usize, Ordering::Relaxed);
            })),
        );
        let mut io_batch = IoBatch::with_capacity(1);
        bio.submit(device, &mut io_batch).unwrap();
        dm.handle_requests();
        let _ = io_batch.wait_all();

        let status = BioStatus::try_from(status.load(Ordering::Relaxed) as u32).unwrap();
        let mut bytes = vec![0u8; len];
        if status == BioStatus::Complete {
            segment.inner_dma_slice().read_bytes(0, &mut bytes).unwrap();
        }
        (status, bytes)
    }

    fn submit_device_io(
        device: &DmDevice,
        type_: BioType,
        offset: usize,
        segments: Vec<BioSegment>,
    ) -> BioStatus {
        let status = Arc::new(AtomicUsize::new(BioStatus::Init as usize));
        let complete_status = status.clone();
        let bio = Bio::new(
            type_,
            Sid::from_offset(offset),
            segments,
            Some(Box::new(move |bio_status| {
                complete_status.store(bio_status as usize, Ordering::Relaxed);
            })),
        );
        let mut io_batch = IoBatch::with_capacity(1);
        bio.submit(device, &mut io_batch).unwrap();
        device.handle_requests();
        let _ = io_batch.wait_all();
        BioStatus::try_from(status.load(Ordering::Relaxed) as u32).unwrap()
    }

    fn read_device(device: &DmDevice, offset: usize, len: usize) -> (BioStatus, Vec<u8>) {
        let segment = BioSegment::alloc(len.div_ceil(BLOCK_SIZE), BioDirection::FromDevice);
        let status = submit_device_io(device, BioType::Read, offset, vec![segment.clone()]);
        let mut bytes = vec![0u8; len];
        if status == BioStatus::Complete {
            segment.inner_dma_slice().read_bytes(0, &mut bytes).unwrap();
        }
        (status, bytes)
    }

    fn write_device(device: &DmDevice, offset: usize, bytes: &[u8]) -> BioStatus {
        let segment = BioSegment::alloc(bytes.len().div_ceil(BLOCK_SIZE), BioDirection::ToDevice);
        segment.inner_dma_slice().write_bytes(0, bytes).unwrap();
        submit_device_io(device, BioType::Write, offset, vec![segment])
    }

    #[ktest]
    fn linear_target_remaps_reads_and_writes() {
        let lower = Arc::new(MemoryDisk::new("linear-lower", 1, 8));
        let source = vec![0x5au8; BLOCK_SIZE];
        lower.write_bytes_at(2 * BLOCK_SIZE, &source);

        let target = LinearTarget::new(lower.clone(), (2 * BLOCK_SIZE / SECTOR_SIZE) as u64);
        let table = DmTable::new(vec![DmTableSegment {
            start_sector: 0,
            len_sectors: (BLOCK_SIZE / SECTOR_SIZE) as u64,
            target: Box::new(target),
        }])
        .unwrap();
        let dm = DmDevice::new("dm-linear-test".into(), DeviceId::null(), table);

        let (status, read_back) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(read_back, source);

        let replacement = vec![0xa5u8; BLOCK_SIZE];
        assert_eq!(write_device(&dm, 0, &replacement), BioStatus::Complete);
        let mut lower_bytes = vec![0u8; BLOCK_SIZE];
        lower.read_bytes_at(2 * BLOCK_SIZE, &mut lower_bytes);
        assert_eq!(lower_bytes, replacement);
    }

    #[ktest]
    fn dm_device_honors_submitted_bio_sid_offset() {
        let lower = Arc::new(MemoryDisk::new("sid-offset-lower", 14, 8));
        let first_block = vec![0x11u8; BLOCK_SIZE];
        let second_block = vec![0x22u8; BLOCK_SIZE];
        lower.write_bytes_at(0, &first_block);
        lower.write_bytes_at(BLOCK_SIZE, &second_block);

        let table = DmTable::new(vec![DmTableSegment {
            start_sector: 0,
            len_sectors: (4 * BLOCK_SIZE / SECTOR_SIZE) as u64,
            target: Box::new(LinearTarget::new(lower.clone(), 0)),
        }])
        .unwrap();
        let dm = Arc::new(DmDevice::new(
            "dm-sid-offset-test".into(),
            DeviceId::null(),
            table,
        ));
        let partition = OffsetBlockDevice {
            name: "dm-sid-offset-test1",
            id: DeviceId::null(),
            inner: dm.clone(),
            sid_offset: (BLOCK_SIZE / SECTOR_SIZE) as u64,
        };

        let (status, read_back) = read_offset_device(&partition, &dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(read_back, second_block);
    }

    #[ktest]
    fn zero_and_error_targets_have_expected_io_behavior() {
        let zero_table = DmTable::new(vec![DmTableSegment {
            start_sector: 0,
            len_sectors: (BLOCK_SIZE / SECTOR_SIZE) as u64,
            target: Box::<ZeroTarget>::default(),
        }])
        .unwrap();
        let zero_dm = DmDevice::new("dm-zero-test".into(), DeviceId::null(), zero_table);
        let (status, read_back) = read_device(&zero_dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(read_back, vec![0u8; BLOCK_SIZE]);

        let error_table = DmTable::new(vec![DmTableSegment {
            start_sector: 0,
            len_sectors: (BLOCK_SIZE / SECTOR_SIZE) as u64,
            target: Box::<ErrorTarget>::default(),
        }])
        .unwrap();
        let error_dm = DmDevice::new("dm-error-test".into(), DeviceId::null(), error_table);
        let (status, _) = read_device(&error_dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
    }
}
