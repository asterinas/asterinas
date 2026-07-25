// SPDX-License-Identifier: MPL-2.0

//! Owns Boot-region loading, validation, volume flags, and mount-time root-directory anchors.
//!
//! This module decodes the exFAT boot region into validated geometry
//! that the rest of the filesystem treats as authoritative.
//! It covers sector and cluster sizing,
//! FAT placement,
//! root-directory anchors,
//! and the volume flags and checksum state needed at mount time.
//!
//! Its entry points load boot sectors from the block device,
//! validate checksum and structural invariants,
//! and expose the derived layout values used by bitmap, FAT, inode, and up-case owners.
//! Recovery here means refusing inconsistent geometry before publication;
//! later modules rely on these fields remaining stable after mount admission.
//!
//! The supported surface is the validated geometry used by this implementation.
//! Malformed flags, impossible sizes, and unsupported layout combinations are rejected
//! instead of normalized heuristically.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 2, 3, and 9.1 through 9.4,
//! plus `aster_block::BlockDevice`.

use aster_block::BlockDevice;
use ostd::mm::VmIo;

use super::{
    bitmap::{ALLOCATION_BITMAP_ENTRY_TYPE, AllocationBitmap},
    device_io,
    fat::{ChainVisitControl, FatReader},
    invalid_on_disk_layout,
    upcase::{UPCASE_TABLE_ENTRY_TYPE, UpcaseTable},
};
use crate::prelude::*;

const END_OF_DIRECTORY_ENTRY_TYPE: u8 = 0x00;
const FAT_ENTRY_SIZE: u64 = size_of::<u32>() as u64;
const FIRST_DATA_CLUSTER: u32 = 2;
const FIRST_FAT_SECTOR: u64 = 24;
const MAX_CLUSTER_SIZE: usize = 32 * 1024 * 1024;
const BOOT_SECTOR_HEADER_LEN: usize = 512;
const FILE_SYSTEM_NAME_OFFSET: usize = 3;
const FILE_SYSTEM_NAME_WIDTH: usize = 8;
const VOLUME_LENGTH_OFFSET: usize = 72;
const VOLUME_LENGTH_WIDTH: usize = 8;
const FAT_OFFSET_OFFSET: usize = 80;
const FAT_LENGTH_OFFSET: usize = 84;
const CLUSTER_HEAP_OFFSET_OFFSET: usize = 88;
const CLUSTER_COUNT_OFFSET: usize = 92;
const ROOT_DIR_CLUSTER_OFFSET: usize = 96;
const VOLUME_SERIAL_NUMBER_OFFSET: usize = 100;
pub(super) const VOLUME_FLAGS_OFFSET: usize = 106;
pub(super) const VOLUME_FLAGS_WIDTH: usize = 2;
const VOLUME_FLAG_VOLUME_DIRTY: u16 = 0x0002;
const VOLUME_FLAG_MEDIA_FAILURE: u16 = 0x0004;
const VOLUME_FLAG_CLEAR_TO_ZERO: u16 = 0x0008;
const BYTES_PER_SECTOR_SHIFT_OFFSET: usize = 108;
const SECTORS_PER_CLUSTER_SHIFT_OFFSET: usize = 109;
const NUMBER_OF_FATS_OFFSET: usize = 110;
const PERCENT_IN_USE_OFFSET: usize = 112;
const BOOT_SIGNATURE_OFFSET: usize = 510;
const BOOT_SIGNATURE_WIDTH: usize = 2;
const U32_FIELD_WIDTH: usize = size_of::<u32>();

#[derive(Clone, Copy)]
pub(super) struct BootRegion {
    pub(super) cluster_count: u32,
    pub(super) cluster_heap_offset_sectors: u32,
    pub(super) cluster_size: usize,
    pub(super) fat_length_sectors: u32,
    pub(super) fat_offset_sectors: u32,
    pub(super) root_dir_cluster: u32,
    pub(super) sector_size: usize,
    pub(super) sectors_per_cluster: usize,
    pub(super) volume_length_sectors: u64,
    pub(super) volume_serial_number: u32,
}

#[derive(Clone, Copy)]
pub(super) struct VolumeFlags {
    pub(super) clear_to_zero: bool,
    pub(super) media_failure: bool,
    pub(super) volume_dirty: bool,
}

impl VolumeFlags {
    pub(super) fn read(block_device: &dyn BlockDevice, boot_region: &BootRegion) -> Result<Self> {
        let mut boot_sector = vec![0; boot_region.sector_size];
        block_device
            .read_bytes(0, &mut boot_sector)
            .map_err(|_| device_io())?;
        let volume_flags = u16::from_le_bytes([
            boot_sector[VOLUME_FLAGS_OFFSET],
            boot_sector[VOLUME_FLAGS_OFFSET + VOLUME_FLAGS_WIDTH - 1],
        ]);
        Ok(Self {
            clear_to_zero: volume_flags & VOLUME_FLAG_CLEAR_TO_ZERO != 0,
            media_failure: volume_flags & VOLUME_FLAG_MEDIA_FAILURE != 0,
            volume_dirty: volume_flags & VOLUME_FLAG_VOLUME_DIRTY != 0,
        })
    }
}

impl BootRegion {
    pub(super) fn load_mount_state(
        block_device: &dyn BlockDevice,
    ) -> Result<(Self, VolumeFlags, AllocationBitmap, Arc<UpcaseTable>)> {
        let boot_region = Self::load(block_device)?;
        let flags = VolumeFlags::read(block_device, &boot_region)?;
        let mut fat_reader = FatReader::new(block_device, &boot_region);
        let (mut bitmap, upcase_entry) = Self::scan_root_directory(&boot_region, &mut fat_reader)?;
        let upcase_table = Arc::new(UpcaseTable::load(
            &boot_region,
            &mut fat_reader,
            upcase_entry,
        )?);
        let used_clusters = bitmap.count_used_clusters(&boot_region, &mut fat_reader)?;
        bitmap.set_used_clusters(used_clusters);
        Ok((boot_region, flags, bitmap, upcase_table))
    }

    fn load(block_device: &dyn BlockDevice) -> Result<Self> {
        let mut sector_header = [0u8; BOOT_SECTOR_HEADER_LEN];
        block_device
            .read_bytes(0, &mut sector_header)
            .map_err(|_| device_io())?;
        if &sector_header[FILE_SYSTEM_NAME_OFFSET..FILE_SYSTEM_NAME_OFFSET + FILE_SYSTEM_NAME_WIDTH]
            != b"EXFAT   "
        {
            return Err(invalid_on_disk_layout());
        }
        if u16::from_le_bytes([
            sector_header[BOOT_SIGNATURE_OFFSET],
            sector_header[BOOT_SIGNATURE_OFFSET + BOOT_SIGNATURE_WIDTH - 1],
        ]) != 0xAA55
        {
            return Err(invalid_on_disk_layout());
        }

        let bytes_per_sector_shift = sector_header[BYTES_PER_SECTOR_SHIFT_OFFSET];
        let sectors_per_cluster_shift = sector_header[SECTORS_PER_CLUSTER_SHIFT_OFFSET];
        if !(9..=12).contains(&bytes_per_sector_shift) {
            return Err(invalid_on_disk_layout());
        }
        let sector_size = 1usize
            .checked_shl(u32::from(bytes_per_sector_shift))
            .ok_or_else(invalid_on_disk_layout)?;
        let sectors_per_cluster = 1usize
            .checked_shl(u32::from(sectors_per_cluster_shift))
            .ok_or_else(invalid_on_disk_layout)?;
        let cluster_size = sector_size
            .checked_mul(sectors_per_cluster)
            .ok_or_else(invalid_on_disk_layout)?;
        if cluster_size == 0 || cluster_size > MAX_CLUSTER_SIZE {
            return Err(invalid_on_disk_layout());
        }
        // TODO: Support clusters smaller than `PAGE_SIZE`. A cached page can then span
        // non-contiguous clusters, requiring sub-page BIO segments or an exFAT bounce buffer.
        if cluster_size < PAGE_SIZE {
            return Err(invalid_on_disk_layout());
        }

        let volume_length_sectors = u64::from_le_bytes([
            sector_header[VOLUME_LENGTH_OFFSET],
            sector_header[VOLUME_LENGTH_OFFSET + 1],
            sector_header[VOLUME_LENGTH_OFFSET + 2],
            sector_header[VOLUME_LENGTH_OFFSET + 3],
            sector_header[VOLUME_LENGTH_OFFSET + 4],
            sector_header[VOLUME_LENGTH_OFFSET + 5],
            sector_header[VOLUME_LENGTH_OFFSET + 6],
            sector_header[VOLUME_LENGTH_OFFSET + VOLUME_LENGTH_WIDTH - 1],
        ]);
        let fat_offset_sectors = u32::from_le_bytes([
            sector_header[FAT_OFFSET_OFFSET],
            sector_header[FAT_OFFSET_OFFSET + 1],
            sector_header[FAT_OFFSET_OFFSET + 2],
            sector_header[FAT_OFFSET_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let fat_length_sectors = u32::from_le_bytes([
            sector_header[FAT_LENGTH_OFFSET],
            sector_header[FAT_LENGTH_OFFSET + 1],
            sector_header[FAT_LENGTH_OFFSET + 2],
            sector_header[FAT_LENGTH_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let cluster_heap_offset_sectors = u32::from_le_bytes([
            sector_header[CLUSTER_HEAP_OFFSET_OFFSET],
            sector_header[CLUSTER_HEAP_OFFSET_OFFSET + 1],
            sector_header[CLUSTER_HEAP_OFFSET_OFFSET + 2],
            sector_header[CLUSTER_HEAP_OFFSET_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let cluster_count = u32::from_le_bytes([
            sector_header[CLUSTER_COUNT_OFFSET],
            sector_header[CLUSTER_COUNT_OFFSET + 1],
            sector_header[CLUSTER_COUNT_OFFSET + 2],
            sector_header[CLUSTER_COUNT_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let root_dir_cluster = u32::from_le_bytes([
            sector_header[ROOT_DIR_CLUSTER_OFFSET],
            sector_header[ROOT_DIR_CLUSTER_OFFSET + 1],
            sector_header[ROOT_DIR_CLUSTER_OFFSET + 2],
            sector_header[ROOT_DIR_CLUSTER_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let volume_serial_number = u32::from_le_bytes([
            sector_header[VOLUME_SERIAL_NUMBER_OFFSET],
            sector_header[VOLUME_SERIAL_NUMBER_OFFSET + 1],
            sector_header[VOLUME_SERIAL_NUMBER_OFFSET + 2],
            sector_header[VOLUME_SERIAL_NUMBER_OFFSET + U32_FIELD_WIDTH - 1],
        ]);
        let number_of_fats = sector_header[NUMBER_OF_FATS_OFFSET];

        if number_of_fats != 1
            || fat_offset_sectors == 0
            || fat_length_sectors == 0
            || cluster_count == 0
        {
            return Err(invalid_on_disk_layout());
        }

        let boot_region = Self {
            cluster_count,
            cluster_heap_offset_sectors,
            cluster_size,
            fat_length_sectors,
            fat_offset_sectors,
            root_dir_cluster,
            sector_size,
            sectors_per_cluster,
            volume_length_sectors,
            volume_serial_number,
        };
        boot_region.validate_geometry()?;
        boot_region.validate_checksum(block_device)?;
        Ok(boot_region)
    }

    pub(super) fn cluster_offset(&self, cluster: u32) -> Result<usize> {
        if !self.is_valid_cluster(cluster) {
            return Err(invalid_on_disk_layout());
        }
        let cluster_index = u64::from(cluster - FIRST_DATA_CLUSTER);
        let sectors_per_cluster =
            u64::try_from(self.sectors_per_cluster).map_err(|_| invalid_on_disk_layout())?;
        let sector_index = cluster_index
            .checked_mul(sectors_per_cluster)
            .and_then(|offset| offset.checked_add(u64::from(self.cluster_heap_offset_sectors)))
            .ok_or_else(invalid_on_disk_layout)?;
        let sector_size = u64::try_from(self.sector_size).map_err(|_| invalid_on_disk_layout())?;
        let byte_offset = sector_index
            .checked_mul(sector_size)
            .ok_or_else(invalid_on_disk_layout)?;
        usize::try_from(byte_offset).map_err(|_| invalid_on_disk_layout())
    }

    pub(super) fn cluster_count_usize(&self) -> Result<usize> {
        usize::try_from(self.cluster_count).map_err(|_| invalid_on_disk_layout())
    }

    pub(super) fn cluster_from_index(&self, cluster_index: usize) -> Result<u32> {
        let cluster_index = u32::try_from(cluster_index).map_err(|_| invalid_on_disk_layout())?;
        let cluster = FIRST_DATA_CLUSTER
            .checked_add(cluster_index)
            .ok_or_else(invalid_on_disk_layout)?;
        if !self.is_valid_cluster(cluster) {
            return Err(invalid_on_disk_layout());
        }
        Ok(cluster)
    }

    pub(super) fn cluster_index(&self, cluster: u32) -> Result<usize> {
        if !self.is_valid_cluster(cluster) {
            return Err(invalid_on_disk_layout());
        }
        usize::try_from(cluster - FIRST_DATA_CLUSTER).map_err(|_| invalid_on_disk_layout())
    }

    pub(super) fn data_capacity_bytes(&self) -> Result<u64> {
        let cluster_size =
            u64::try_from(self.cluster_size).map_err(|_| invalid_on_disk_layout())?;
        u64::from(self.cluster_count)
            .checked_mul(cluster_size)
            .ok_or_else(invalid_on_disk_layout)
    }

    pub(super) fn is_valid_cluster(&self, cluster: u32) -> bool {
        cluster >= FIRST_DATA_CLUSTER
            && cluster <= self.cluster_count.saturating_add(FIRST_DATA_CLUSTER - 1)
    }

    pub(super) fn validate_stream_data(&self, first_cluster: u32, data_length: u64) -> Result<()> {
        if !self.is_valid_cluster(first_cluster) || data_length == 0 {
            return Err(invalid_on_disk_layout());
        }
        if data_length > self.data_capacity_bytes()? {
            return Err(invalid_on_disk_layout());
        }
        Ok(())
    }

    fn validate_checksum(&self, block_device: &dyn BlockDevice) -> Result<()> {
        let checksum_region_len = self
            .sector_size
            .checked_mul(11)
            .ok_or_else(invalid_on_disk_layout)?;
        let mut checksum_region = vec![0; checksum_region_len];
        block_device
            .read_bytes(0, &mut checksum_region)
            .map_err(|_| device_io())?;
        let expected_checksum = Self::checksum(&checksum_region);

        let mut checksum_sector = vec![0; self.sector_size];
        block_device
            .read_bytes(checksum_region_len, &mut checksum_sector)
            .map_err(|_| device_io())?;
        for chunk in checksum_sector.as_chunks::<{ size_of::<u32>() }>().0 {
            if u32::from_le_bytes(*chunk) != expected_checksum {
                return Err(invalid_on_disk_layout());
            }
        }
        Ok(())
    }

    fn validate_geometry(&self) -> Result<()> {
        if !self.is_valid_cluster(self.root_dir_cluster) {
            return Err(invalid_on_disk_layout());
        }
        let sector_size = u64::try_from(self.sector_size).map_err(|_| invalid_on_disk_layout())?;
        let data_sectors = u64::from(self.cluster_count)
            .checked_mul(
                u64::try_from(self.sectors_per_cluster).map_err(|_| invalid_on_disk_layout())?,
            )
            .ok_or_else(invalid_on_disk_layout)?;
        let heap_end = u64::from(self.cluster_heap_offset_sectors)
            .checked_add(data_sectors)
            .ok_or_else(invalid_on_disk_layout)?;
        if heap_end > self.volume_length_sectors {
            return Err(invalid_on_disk_layout());
        }
        let fat_end = u64::from(self.fat_offset_sectors)
            .checked_add(u64::from(self.fat_length_sectors))
            .ok_or_else(invalid_on_disk_layout)?;
        let fat_entry_count = u64::from(self.cluster_count)
            .checked_add(u64::from(FIRST_DATA_CLUSTER))
            .ok_or_else(invalid_on_disk_layout)?;
        let required_fat_bytes = fat_entry_count
            .checked_mul(FAT_ENTRY_SIZE)
            .ok_or_else(invalid_on_disk_layout)?;
        let fat_bytes = u64::from(self.fat_length_sectors)
            .checked_mul(sector_size)
            .ok_or_else(invalid_on_disk_layout)?;
        if u64::from(self.fat_offset_sectors) < FIRST_FAT_SECTOR
            || fat_end > u64::from(self.cluster_heap_offset_sectors)
            || fat_bytes < required_fat_bytes
        {
            return Err(invalid_on_disk_layout());
        }
        Ok(())
    }

    fn checksum(bytes: &[u8]) -> u32 {
        let mut checksum = 0u32;
        for (offset, byte) in bytes.iter().enumerate() {
            if (VOLUME_FLAGS_OFFSET..VOLUME_FLAGS_OFFSET + VOLUME_FLAGS_WIDTH).contains(&offset)
                || offset == PERCENT_IN_USE_OFFSET
            {
                continue;
            }
            checksum = checksum.rotate_right(1).wrapping_add(u32::from(*byte));
        }
        checksum
    }

    fn scan_root_directory(
        boot_region: &BootRegion,
        fat_reader: &mut FatReader<'_>,
    ) -> Result<(AllocationBitmap, [u8; 32])> {
        let mut bitmap = None;
        let mut upcase_entry = None;
        fat_reader.walk_cluster_chain(boot_region.root_dir_cluster, |_, cluster_bytes| {
            for entry in cluster_bytes.as_chunks::<32>().0 {
                match entry[0] {
                    END_OF_DIRECTORY_ENTRY_TYPE => return Ok(ChainVisitControl::Stop),
                    ALLOCATION_BITMAP_ENTRY_TYPE => bitmap = Some(AllocationBitmap::parse(entry)?),
                    UPCASE_TABLE_ENTRY_TYPE => {
                        upcase_entry = Some(
                            <[u8; 32]>::try_from(&entry[..])
                                .map_err(|_| invalid_on_disk_layout())?,
                        );
                    }
                    _ => (),
                }
                if bitmap.is_some() && upcase_entry.is_some() {
                    return Ok(ChainVisitControl::Stop);
                }
            }
            Ok(ChainVisitControl::Continue)
        })?;
        Ok((
            bitmap.ok_or_else(invalid_on_disk_layout)?,
            upcase_entry.ok_or_else(invalid_on_disk_layout)?,
        ))
    }

    pub(super) fn write_volume_flags(
        &self,
        block_device: &dyn BlockDevice,
        flags: VolumeFlags,
    ) -> Result<()> {
        let mut boot_sector = vec![0; self.sector_size];
        block_device
            .read_bytes(0, &mut boot_sector)
            .map_err(|_| device_io())?;

        let mut volume_flags = 0u16;
        if flags.volume_dirty {
            volume_flags |= VOLUME_FLAG_VOLUME_DIRTY;
        }
        if flags.media_failure {
            volume_flags |= VOLUME_FLAG_MEDIA_FAILURE;
        }
        if flags.clear_to_zero {
            volume_flags |= VOLUME_FLAG_CLEAR_TO_ZERO;
        }
        boot_sector[VOLUME_FLAGS_OFFSET..VOLUME_FLAGS_OFFSET + VOLUME_FLAGS_WIDTH]
            .copy_from_slice(&volume_flags.to_le_bytes());

        block_device
            .write_bytes(0, &boot_sector)
            .map_err(|_| device_io())?;
        Ok(())
    }
}
