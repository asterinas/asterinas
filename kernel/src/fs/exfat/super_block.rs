// SPDX-License-Identifier: MPL-2.0

use ostd::Pod;

use super::constants::{EXFAT_FIRST_CLUSTER, EXFAT_RESERVED_CLUSTERS, MEDIA_FAILURE, VOLUME_DIRTY};
use crate::prelude::*;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
// The in-memory superblock info
pub struct ExfatSuperBlock {
    /// num of sectors in volume
    pub num_sectors: u64,
    /// num of clusters in volume
    pub num_clusters: u32,
    /// sector size in bytes
    pub sector_size: u32,
    /// cluster size in bytes
    pub cluster_size: u32,
    pub cluster_size_bits: u32,
    /// cluster size in sectors
    pub sect_per_cluster: u32,
    pub sect_per_cluster_bits: u32,
    /// FAT1 start sector
    pub fat1_start_sector: u64,
    /// FAT2 start sector
    pub fat2_start_sector: u64,
    /// data area start sector
    pub data_start_sector: u64,
    /// number of FAT sectors
    pub num_fat_sectors: u32,
    /// root dir cluster
    pub root_dir: u32,
    /// number of dentries per cluster
    pub dentries_per_clu: u32,
    /// volume flags
    pub vol_flags: u32,
    /// volume flags to retain
    pub vol_flags_persistent: u32,
    /// cluster search pointer
    pub cluster_search_ptr: u32,
    /// number of used clusters
    pub used_clusters: u32,
}

const DENTRY_SIZE_BITS: u32 = 5;

impl TryFrom<ExfatBootSector> for ExfatSuperBlock {
    type Error = crate::error::Error;
    fn try_from(sector: ExfatBootSector) -> Result<ExfatSuperBlock> {
        const EXFAT_CLUSTERS_UNTRACKED: u32 = !0;
        let mut block = ExfatSuperBlock {
            sect_per_cluster_bits: sector.sector_per_cluster_bits as u32,
            sect_per_cluster: 1 << sector.sector_per_cluster_bits as u32,

            cluster_size_bits: (sector.sector_per_cluster_bits + sector.sector_size_bits) as u32,
            cluster_size: 1 << (sector.sector_per_cluster_bits + sector.sector_size_bits) as u32,

            sector_size: 1 << sector.sector_size_bits,
            num_fat_sectors: sector.fat_length,
            fat1_start_sector: sector.fat_offset as u64,
            fat2_start_sector: sector.fat_offset as u64,

            data_start_sector: sector.cluster_offset as u64,
            num_sectors: sector.vol_length,
            num_clusters: sector.cluster_count + EXFAT_RESERVED_CLUSTERS,

            root_dir: sector.root_cluster,

            vol_flags: sector.vol_flags as u32,
            vol_flags_persistent: (sector.vol_flags & (VOLUME_DIRTY | MEDIA_FAILURE)) as u32,

            cluster_search_ptr: EXFAT_FIRST_CLUSTER,

            used_clusters: EXFAT_CLUSTERS_UNTRACKED,

            dentries_per_clu: 1
                << ((sector.sector_per_cluster_bits + sector.sector_size_bits) as u32
                    - DENTRY_SIZE_BITS),
        };

        if block.num_fat_sectors == 2 {
            block.fat2_start_sector += block.num_fat_sectors as u64;
        }

        Ok(block)
    }
}

pub const BOOTSEC_JUMP_BOOT_LEN: usize = 3;
pub const BOOTSEC_FS_NAME_LEN: usize = 8;
pub const BOOTSEC_OLDBPB_LEN: usize = 53;
// EXFAT: Main and Backup Boot Sector (512 bytes)
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct ExfatBootSector {
    pub jmp_boot: [u8; BOOTSEC_JUMP_BOOT_LEN],
    pub fs_name: [u8; BOOTSEC_FS_NAME_LEN],
    pub must_be_zero: [u8; BOOTSEC_OLDBPB_LEN],
    pub partition_offset: u64,
    pub vol_length: u64,
    pub fat_offset: u32,
    pub fat_length: u32,
    pub cluster_offset: u32,
    pub cluster_count: u32,
    pub root_cluster: u32,
    pub vol_serial: u32,
    pub fs_revision: [u8; 2],
    pub vol_flags: u16,
    pub sector_size_bits: u8,
    pub sector_per_cluster_bits: u8,
    pub num_fats: u8,
    pub drv_sel: u8,
    pub percent_in_use: u8,
    pub reserved: [u8; 7],
    pub boot_code: [u8; 390],
    pub signature: u16,
}
