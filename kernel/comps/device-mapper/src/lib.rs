// SPDX-License-Identifier: MPL-2.0

//! Device-mapper support for Asterinas.
//!
//! This component implements a table-driven virtual block-device layer inspired
//! by Linux device-mapper. It supports `linear`, `zero`, `error`, and read-only
//! `verity` targets created from `dm-mod.create=`/`dm_mod.create=` kernel
//! command-line entries.
//!
//! References:
//! - Linux device-mapper documentation:
//!   <https://docs.kernel.org/admin-guide/device-mapper/>
//! - Linux dm-verity documentation:
//!   <https://docs.kernel.org/admin-guide/device-mapper/verity.html>

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
mod sha256;
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
    use alloc::{
        boxed::Box,
        string::{String, ToString},
        sync::Arc,
        vec,
        vec::Vec,
    };
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
        DmError,
        device::DmDevice,
        table::{DmTable, DmTableSegment},
        target::{
            error::ErrorTarget,
            linear::LinearTarget,
            verity::{VerityTarget, build_hash_levels},
            zero::ZeroTarget,
        },
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

    struct RegisteredDiskGuard(DeviceId);

    impl Drop for RegisteredDiskGuard {
        fn drop(&mut self) {
            let _ = aster_block::unregister(self.0);
        }
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

    fn digest_v1(block: &[u8], salt: &[u8]) -> [u8; 32] {
        super::sha256::digest(&[salt, block])
    }

    fn install_verity_tree(
        data_disk: &MemoryDisk,
        hash_disk: &MemoryDisk,
        data_blocks: &[Vec<u8>],
        salt: &[u8],
        hash_start_block: u64,
    ) -> [u8; 32] {
        for (index, block) in data_blocks.iter().enumerate() {
            data_disk.write_bytes_at(index * BLOCK_SIZE, block);
        }

        let hashes_per_block = BLOCK_SIZE / 32;
        let levels = build_hash_levels(
            data_blocks.len() as u64,
            hashes_per_block as u64,
            hash_start_block,
        )
        .unwrap();
        let mut child_hashes: Vec<[u8; 32]> = data_blocks
            .iter()
            .map(|block| digest_v1(block, salt))
            .collect();
        let leaf_to_root: Vec<_> = levels.iter().rev().copied().collect();
        for level in leaf_to_root {
            let mut next_hashes = Vec::new();
            for block_index in 0..level.nr_blocks as usize {
                let mut hash_block = vec![0u8; BLOCK_SIZE];
                let start = block_index * hashes_per_block;
                let end = (start + hashes_per_block).min(child_hashes.len());
                for (slot, digest) in child_hashes[start..end].iter().enumerate() {
                    hash_block[slot * 32..slot * 32 + 32].copy_from_slice(digest);
                }
                hash_disk.write_bytes_at(
                    (level.first_block as usize + block_index) * BLOCK_SIZE,
                    &hash_block,
                );
                next_hashes.push(digest_v1(&hash_block, salt));
            }
            child_hashes = next_hashes;
        }
        child_hashes[0]
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

    fn to_hex(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|byte| alloc::format!("{:02x}", byte))
            .collect()
    }

    /// Builds a verity `DmDevice` over already-registered lower devices, going
    /// through the public table-argument parser so the tests exercise the same
    /// path as the `dm-mod.create=` command line.
    fn build_verity_device(
        data_name: &str,
        hash_name: &str,
        num_data_blocks: usize,
        root_hex: &str,
        salt_hex: &str,
    ) -> DmDevice {
        let verity = VerityTarget::from_table_args(&[
            "1",
            data_name,
            hash_name,
            &BLOCK_SIZE.to_string(),
            &BLOCK_SIZE.to_string(),
            &num_data_blocks.to_string(),
            "0",
            "sha256",
            root_hex,
            salt_hex,
        ])
        .unwrap();
        let table = DmTable::new(vec![DmTableSegment {
            start_sector: 0,
            len_sectors: (num_data_blocks * BLOCK_SIZE / SECTOR_SIZE) as u64,
            target: Box::new(verity),
        }])
        .unwrap();
        DmDevice::new("dm-verity-test".into(), DeviceId::null(), table)
    }

    #[ktest]
    fn verity_target_detects_data_corruption() {
        let data_disk = Arc::new(MemoryDisk::new("verity-data", 2, 4));
        let hash_disk = Arc::new(MemoryDisk::new("verity-hash", 3, 4));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());
        let data_blocks = vec![
            vec![0x11u8; BLOCK_SIZE],
            vec![0x22u8; BLOCK_SIZE],
            vec![0x33u8; BLOCK_SIZE],
        ];
        let salt = [0x7bu8; 16];
        let root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);
        let dm = build_verity_device(
            "verity-data",
            "verity-hash",
            data_blocks.len(),
            &to_hex(&root_digest),
            &to_hex(&salt),
        );

        let (status, read_back) = read_device(&dm, BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(read_back, data_blocks[1]);

        data_disk.write_bytes_at(BLOCK_SIZE, &vec![0xffu8; BLOCK_SIZE]);
        let (status, _) = read_device(&dm, BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
    }

    #[ktest]
    fn verity_target_detects_hash_corruption() {
        let data_disk = Arc::new(MemoryDisk::new("verity-hc-data", 6, 4));
        let hash_disk = Arc::new(MemoryDisk::new("verity-hc-hash", 7, 4));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());
        let data_blocks = vec![
            vec![0x11u8; BLOCK_SIZE],
            vec![0x22u8; BLOCK_SIZE],
            vec![0x33u8; BLOCK_SIZE],
        ];
        let salt = [0x7bu8; 16];
        let root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);
        let dm = build_verity_device(
            "verity-hc-data",
            "verity-hc-hash",
            data_blocks.len(),
            &to_hex(&root_digest),
            &to_hex(&salt),
        );

        let (status, _) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);

        // Flip a byte inside the hash tree node. The stored digest no longer
        // matches the data block's hash, so the read must fail.
        let mut hash_byte = [0u8; 1];
        hash_disk.read_bytes_at(0, &mut hash_byte);
        hash_byte[0] ^= 0xff;
        hash_disk.write_bytes_at(0, &hash_byte);
        let (status, _) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
    }

    #[ktest]
    fn verity_target_rejects_wrong_root_digest() {
        let data_disk = Arc::new(MemoryDisk::new("verity-wr-data", 8, 4));
        let hash_disk = Arc::new(MemoryDisk::new("verity-wr-hash", 9, 4));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());
        let data_blocks = vec![vec![0x11u8; BLOCK_SIZE], vec![0x22u8; BLOCK_SIZE]];
        let salt = [0x7bu8; 16];
        let _root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);

        // A device configured with the wrong root digest must reject otherwise
        // intact data, since the on-disk tree no longer chains to the expected
        // root of trust.
        let wrong_root = [0u8; 32];
        let dm = build_verity_device(
            "verity-wr-data",
            "verity-wr-hash",
            data_blocks.len(),
            &to_hex(&wrong_root),
            &to_hex(&salt),
        );
        let (status, _) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
    }

    #[ktest]
    fn verity_target_rejects_undersized_lower_devices() {
        let data_disk = Arc::new(MemoryDisk::new("verity-small-data", 12, 1));
        let hash_disk = Arc::new(MemoryDisk::new("verity-small-hash", 13, 1));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());

        let root_hex = to_hex(&[0u8; 32]);
        let salt_hex = to_hex(&[0x7bu8; 16]);
        let result = VerityTarget::from_table_args(&[
            "1",
            "verity-small-data",
            "verity-small-hash",
            &BLOCK_SIZE.to_string(),
            &BLOCK_SIZE.to_string(),
            "2",
            "0",
            "sha256",
            &root_hex,
            &salt_hex,
        ]);
        assert_eq!(result.unwrap_err().kind, DmError::InvalidArgument);

        let result = VerityTarget::from_table_args(&[
            "1",
            "verity-small-data",
            "verity-small-hash",
            &BLOCK_SIZE.to_string(),
            &BLOCK_SIZE.to_string(),
            "1",
            "1",
            "sha256",
            &root_hex,
            &salt_hex,
        ]);
        assert_eq!(result.unwrap_err().kind, DmError::InvalidArgument);
    }

    #[ktest]
    fn verity_target_accepts_zero_optional_params() {
        let data_disk = Arc::new(MemoryDisk::new("verity-opt-data", 15, 2));
        let hash_disk = Arc::new(MemoryDisk::new("verity-opt-hash", 16, 2));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());

        let data_blocks = vec![vec![0x11u8; BLOCK_SIZE]];
        let salt = [0x7bu8; 16];
        let root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);
        let root_hex = to_hex(&root_digest);
        let salt_hex = to_hex(&salt);
        let target = VerityTarget::from_table_args(&[
            "1",
            "verity-opt-data",
            "verity-opt-hash",
            &BLOCK_SIZE.to_string(),
            &BLOCK_SIZE.to_string(),
            "1",
            "0",
            "sha256",
            &root_hex,
            &salt_hex,
            "0",
        ]);
        assert!(target.is_ok());
    }

    #[ktest]
    fn verity_target_rejects_nonzero_optional_params() {
        let result = VerityTarget::from_table_args(&[
            "1",
            "missing-data",
            "missing-hash",
            &BLOCK_SIZE.to_string(),
            &BLOCK_SIZE.to_string(),
            "1",
            "0",
            "sha256",
            &to_hex(&[0u8; 32]),
            "-",
            "1",
        ]);
        assert_eq!(result.unwrap_err().kind, DmError::UnsupportedTarget);
    }

    #[ktest]
    fn verity_target_verifies_multi_level_tree() {
        // 130 data blocks force a two-level hash tree (128 digests fit in one
        // 4096-byte hash block), exercising the level-by-level walk that the
        // single-block-tree fixtures above do not reach.
        let data_disk = Arc::new(MemoryDisk::new("verity-ml-data", 10, 130));
        let hash_disk = Arc::new(MemoryDisk::new("verity-ml-hash", 11, 8));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());

        let data_blocks: Vec<Vec<u8>> = (0..130)
            .map(|index| vec![index as u8; BLOCK_SIZE])
            .collect();
        let salt = [0x7bu8; 16];
        let root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);
        let dm = build_verity_device(
            "verity-ml-data",
            "verity-ml-hash",
            data_blocks.len(),
            &to_hex(&root_digest),
            &to_hex(&salt),
        );

        // Blocks under different leaf hash blocks both chain to the root.
        let (status, block0) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(block0, data_blocks[0]);
        let (status, block129) = read_device(&dm, 129 * BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(block129, data_blocks[129]);

        // Corrupting a block under the second leaf hash block fails only that
        // block; block 0, under the first leaf, is still served.
        data_disk.write_bytes_at(129 * BLOCK_SIZE, &vec![0xffu8; BLOCK_SIZE]);
        let (status, _) = read_device(&dm, 129 * BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
        let (status, _) = read_device(&dm, 0, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
    }

    #[ktest]
    fn parse_create_arg_builds_verity_device_and_detects_corruption() {
        // This test drives the exact pipeline used at boot: a `dm-mod.create=`
        // table string is parsed into a `DmTable`, the verity target resolves
        // its lower devices by name through the block registry, and a read is
        // served only when the Merkle tree verifies.
        let data_disk = Arc::new(MemoryDisk::new("dm-it-data", 4, 4));
        let hash_disk = Arc::new(MemoryDisk::new("dm-it-hash", 5, 4));
        aster_block::register(data_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        aster_block::register(hash_disk.clone() as Arc<dyn BlockDevice>).unwrap();
        let _data_guard = RegisteredDiskGuard(data_disk.id());
        let _hash_guard = RegisteredDiskGuard(hash_disk.id());

        let data_blocks = vec![
            vec![0x11u8; BLOCK_SIZE],
            vec![0x22u8; BLOCK_SIZE],
            vec![0x33u8; BLOCK_SIZE],
        ];
        let salt = [0x7bu8; 16];
        let root_digest = install_verity_tree(&data_disk, &hash_disk, &data_blocks, &salt, 0);

        let sectors = data_blocks.len() * BLOCK_SIZE / SECTOR_SIZE;
        let table = alloc::format!(
            "dm-verity-it: 0 {} verity 1 dm-it-data dm-it-hash {} {} {} 0 sha256 {} {}",
            sectors,
            BLOCK_SIZE,
            BLOCK_SIZE,
            data_blocks.len(),
            to_hex(&root_digest),
            to_hex(&salt),
        );
        // The boot path receives the value with the surrounding command-line
        // quotes, so verify the quote-stripping and parsing both behave.
        let arg = alloc::format!("\"{}\"", table)
            .parse::<super::DmCreateArg>()
            .unwrap();
        let parsed = super::parser::parse_create_arg(arg.as_str(), 0).unwrap();
        assert_eq!(parsed.name, "dm-verity-it");

        let dm = DmDevice::new(parsed.name.clone(), DeviceId::null(), parsed.table);

        let (status, read_back) = read_device(&dm, BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::Complete);
        assert_eq!(read_back, data_blocks[1]);

        data_disk.write_bytes_at(BLOCK_SIZE, &vec![0xffu8; BLOCK_SIZE]);
        let (status, _) = read_device(&dm, BLOCK_SIZE, BLOCK_SIZE);
        assert_eq!(status, BioStatus::IoError);
    }
}
