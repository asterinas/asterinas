use core::mem::size_of;
use jinux_frame::vm::VmFrame;

use super::super_block::ExfatSuperBlock;
use super::{bitmap::EXFAT_RESERVED_CLUSTERS, fs::ExfatFS};
use crate::prelude::*;

pub type ClusterID = u32;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FatValue {
    Free,
    Next(ClusterID),
    Bad,
    EndOfChain,
}

pub const EXFAT_EOF_CLUSTER: ClusterID = 0xFFFFFFFF;
pub const EXFAT_BAD_CLUSTER: ClusterID = 0xFFFFFFF7;
pub const EXFAT_FREE_CLUSTER: ClusterID = 0;
pub const FAT_ENTRY_SIZE: usize = size_of::<ClusterID>();

impl From<ClusterID> for FatValue {
    fn from(value: ClusterID) -> Self {
        match value {
            EXFAT_BAD_CLUSTER => FatValue::Bad,
            EXFAT_FREE_CLUSTER => FatValue::Free,
            EXFAT_EOF_CLUSTER => FatValue::EndOfChain,
            _ => FatValue::Next(value),
        }
    }
}

impl From<FatValue> for ClusterID {
    fn from(val: FatValue) -> Self {
        match val {
            FatValue::Free => EXFAT_FREE_CLUSTER,
            FatValue::EndOfChain => EXFAT_EOF_CLUSTER,
            FatValue::Bad => EXFAT_BAD_CLUSTER,
            FatValue::Next(x) => x,
        }
    }
}

//FIXME: Should we implement fat as a trait of file system, or as a member of file system?
pub trait FatTrait {
    fn read_next_fat(&self, cluster: ClusterID) -> Result<FatValue>;
    fn write_next_fat(&self, cluster: ClusterID, value: FatValue) -> Result<()>;
}

impl FatTrait for ExfatFS {
    fn read_next_fat(&self, cluster: ClusterID) -> Result<FatValue> {
        let sb: ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        if !self.is_valid_cluster(cluster) {
            return_errno_with_message!(Errno::EIO, "invalid access to FAT")
        }

        let position =
            sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENTRY_SIZE as u64;
        let mut buf: [u8; FAT_ENTRY_SIZE] = [0; FAT_ENTRY_SIZE];
        self.block_device().read_at(position as usize, &mut buf)?;

        let value = u32::from_le_bytes(buf);
        Ok(FatValue::from(value))
    }

    fn write_next_fat(&self, cluster: ClusterID, value: FatValue) -> Result<()> {
        let sb: ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        let position =
            sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENTRY_SIZE as u64;
        let raw_value: u32 = value.into();

        //TODO: should make sure that the write is synchronous.
        self.block_device()
            .write_at(position as usize, &raw_value.to_le_bytes())?;

        if sb.fat1_start_sector != sb.fat2_start_sector {
            let mirror_position = sb.fat2_start_sector * sector_size as u64
                + (cluster as u64) * FAT_ENTRY_SIZE as u64;
            self.block_device()
                .write_at(mirror_position as usize, &raw_value.to_le_bytes())?;
        }

        Ok(())
    }
}

bitflags! {
    #[derive(Default)]
    pub struct FatChainFlags:u8 {
        //An associated allocation of clusters is possible
        const ALLOC_POSSIBLE = 0x01;
        //The allocated clusters are contiguous and fat table is irrevalent.
        const FAT_CHAIN_NOT_IN_USE = 0x03;
    }
}

// Directory pub structures
#[derive(Debug, Clone, Default)]
pub struct ExfatChain {
    // current clusterID
    current: ClusterID,
    // use FAT or not
    flags: FatChainFlags,

    fs: Weak<ExfatFS>,
}

//A position by the chain and relative offset in the cluster.
pub type ExfatChainPosition = (ExfatChain, usize);

impl ExfatChain {
    pub fn new(fs: Weak<ExfatFS>, current: ClusterID, flags: FatChainFlags) -> Self {
        Self { current, flags, fs }
    }

    pub fn cluster_size(&self) -> usize {
        self.fs().cluster_size()
    }

    pub fn cluster_id(&self) -> ClusterID {
        self.current
    }

    pub fn flags(&self) -> FatChainFlags {
        self.flags
    }

    pub(super) fn set_flags(&mut self, flags: FatChainFlags) {
        self.flags = flags;
    }

    fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    pub(super) fn physical_cluster_start_offset(&self) -> usize {
        let cluster_num = (self.current - EXFAT_RESERVED_CLUSTERS) as usize;
        (cluster_num * self.cluster_size())
            + self.fs().super_block().data_start_sector as usize
                * self.fs().super_block().sector_size as usize
    }

    //Walk to the cluster at the given offset, return the new relative offset
    pub fn walk_to_cluster_at_offset(&self, offset: usize) -> Result<ExfatChainPosition> {
        let cluster_size = self.fs().cluster_size();
        let steps = offset / cluster_size;
        let result_chain = self.walk(steps as u32)?;
        let result_offset = offset % cluster_size;
        Ok((result_chain, result_offset))
    }

    pub fn is_next_cluster_eof(&self) -> Result<bool> {
        let fat = self.fs().read_next_fat(self.current)?;
        Ok(matches!(fat, FatValue::EndOfChain))
    }

    pub fn is_current_cluster_valid(&self) -> bool {
        self.fs().is_valid_cluster(self.current)
    }

    //The destination cluster must be a valid cluster.
    pub fn walk(&self, steps: u32) -> Result<ExfatChain> {
        let mut result_cluster = self.current;
        if self.flags.contains(FatChainFlags::FAT_CHAIN_NOT_IN_USE) {
            result_cluster = (result_cluster + steps) as ClusterID;
        } else {
            for _ in 0..steps {
                let fat = self.fs().read_next_fat(result_cluster)?;
                match fat {
                    FatValue::Next(next_fat) => result_cluster = next_fat,
                    _ => return_errno_with_message!(Errno::EIO, "invalid access to FAT cluster"),
                }
            }
        }
        Ok(ExfatChain::new(self.fs.clone(), result_cluster, self.flags))
    }

    //FIXME: What if cluster size is smaller than page size?

    ///Offset must be inside this cluster
    pub fn read_page(&self, offset: usize, page: &VmFrame) -> Result<()> {
        if offset + PAGE_SIZE >= self.cluster_size() {
            return_errno_with_message!(Errno::EINVAL, "wrong offset")
        }

        let physical_offset = self.physical_cluster_start_offset() + offset;
        self.fs()
            .block_device()
            .read_page(physical_offset / PAGE_SIZE, page)
    }

    ///Offset must be inside this cluster
    pub fn write_page(&self, offset: usize, page: &VmFrame) -> Result<()> {
        if offset + PAGE_SIZE >= self.cluster_size() {
            return_errno_with_message!(Errno::EINVAL, "wrong offset")
        }

        let physical_offset = self.physical_cluster_start_offset() + offset;
        self.fs()
            .block_device()
            .write_page(physical_offset / PAGE_SIZE, page)
    }

    //FIXME: Code repetition for read_at and write_at.
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let (mut chain, mut off_in_cluster) = self.walk_to_cluster_at_offset(offset)?;
        let mut bytes_read = 0usize;

        while bytes_read < buf.len() {
            let physical_offset = chain.physical_cluster_start_offset() + off_in_cluster;
            let to_read_size = (buf.len() - bytes_read).min(self.cluster_size() - off_in_cluster);

            let read_size = self.fs().block_device().read_at(
                physical_offset,
                &mut buf[bytes_read..bytes_read + to_read_size],
            )?;

            bytes_read += read_size;
            off_in_cluster += read_size;

            if off_in_cluster == self.cluster_size() {
                chain = chain.walk(1)?;
                off_in_cluster = 0;
            }
        }

        Ok(bytes_read)
    }
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let (mut chain, mut off_in_cluster) = self.walk_to_cluster_at_offset(offset)?;
        let mut bytes_written = 0usize;

        while bytes_written < buf.len() {
            let physical_offset = chain.physical_cluster_start_offset() + off_in_cluster;
            let to_write_size =
                (buf.len() - bytes_written).min(self.cluster_size() - off_in_cluster);

            let write_size = self.fs().block_device().write_at(
                physical_offset,
                &buf[bytes_written..bytes_written + to_write_size],
            )?;

            bytes_written += write_size;
            off_in_cluster += write_size;

            if off_in_cluster == self.cluster_size() {
                chain = chain.walk(1)?;
                off_in_cluster = 0;
            }
        }

        Ok(bytes_written)
    }
}
