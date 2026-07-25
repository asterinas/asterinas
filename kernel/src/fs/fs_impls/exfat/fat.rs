// SPDX-License-Identifier: MPL-2.0

//! Reads and mutates exFAT FAT chains for cluster traversal and topology updates.
//!
//! This module owns exFAT FAT entry decoding and the traversal/linking operations
//! that turn on-disk FAT words into cluster-chain topology.
//! It is the shared owner for walking allocated chains,
//! appending or rewriting links during growth,
//! and validating that chain progress stays within the mounted boot geometry.
//!
//! Its public surface to sibling modules is the FAT reader/writer machinery
//! used by boot loading, directory growth, file growth, and up-case loading.
//! Allocation-sensitive callers pair these operations with allocation guards
//! so that bitmap and FAT publication remain ordered.
//!
//! Locking matters because FAT mutation must not race bitmap accounting
//! or inode-visible cluster-map publication.
//! Recovery paths preserve the distinction between read-only traversal failure,
//! mutation failure before publication,
//! and forced-shutdown-worthy writeback loss.
//!
//! This module is limited to exFAT FAT semantics.
//! It does not own higher-level inode or namespace policy,
//! and it rejects impossible cluster values or malformed chains instead of guessing.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 4, 5.1, and 8.1,
//! plus `aster_block::BlockDevice`.

use alloc::vec;
use core::ops::Range;

use aster_block::BlockDevice;
use ostd::mm::VmIo;

use super::{boot::BootRegion, device_io, invalid_on_disk_layout, invalid_operation_input};
use crate::prelude::*;

const FAT_BAD_CLUSTER: u32 = 0xFFFF_FFF7;
const FAT_END_OF_CHAIN_MIN: u32 = 0xFFFF_FFF8;
const FAT_END_OF_CHAIN: u32 = 0xFFFF_FFFF;
const FAT_ENTRY_SIZE: u64 = size_of::<u32>() as u64;

#[derive(Clone, Copy)]
pub(super) enum ChainVisitControl {
    Continue,
    Stop,
}

#[derive(Clone, Copy)]
pub(super) enum FatChainStep {
    Continue(u32),
    End,
}

pub(super) struct FatReader<'a> {
    block_device: &'a dyn BlockDevice,
    boot_region: &'a BootRegion,
    cached_sector_index: Option<u64>,
    cached_sector: Vec<u8>,
}

impl<'a> FatReader<'a> {
    pub(super) fn new(block_device: &'a dyn BlockDevice, boot_region: &'a BootRegion) -> Self {
        Self {
            block_device,
            boot_region,
            cached_sector_index: None,
            cached_sector: vec![0; boot_region.sector_size],
        }
    }

    pub(super) fn walk_cluster_chain<F>(
        &mut self,
        start_cluster: u32,
        mut visit_cluster_fn: F,
    ) -> Result<()>
    where
        F: FnMut(u32, &[u8]) -> Result<ChainVisitControl>,
    {
        if !self.boot_region.is_valid_cluster(start_cluster) {
            return Err(invalid_on_disk_layout());
        }
        let mut cluster_buffer = vec![0; self.boot_region.cluster_size];
        let mut current_cluster = start_cluster;
        let mut visited_clusters = BTreeSet::new();
        loop {
            if !visited_clusters.insert(current_cluster) {
                return Err(invalid_on_disk_layout());
            }
            let cluster_offset = self.boot_region.cluster_offset(current_cluster)?;
            self.block_device
                .read_bytes(cluster_offset, &mut cluster_buffer)
                .map_err(|_| device_io())?;
            if matches!(
                visit_cluster_fn(current_cluster, &cluster_buffer)?,
                ChainVisitControl::Stop
            ) {
                return Ok(());
            }
            current_cluster = match self.next_cluster(current_cluster)? {
                FatChainStep::Continue(next_cluster) => next_cluster,
                FatChainStep::End => return Ok(()),
            };
        }
    }

    pub(super) fn next_cluster(&mut self, current_cluster: u32) -> Result<FatChainStep> {
        let (_, entry_range) = self.cached_entry_range_for_cluster(current_cluster)?;
        let next_cluster = {
            let entry = self
                .cached_sector
                .get(entry_range)
                .ok_or_else(invalid_on_disk_layout)?;
            u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]])
        };
        if next_cluster == FAT_BAD_CLUSTER {
            return Err(invalid_on_disk_layout());
        }
        if next_cluster >= FAT_END_OF_CHAIN_MIN {
            return Ok(FatChainStep::End);
        }
        if !self.boot_region.is_valid_cluster(next_cluster) {
            return Err(invalid_on_disk_layout());
        }
        Ok(FatChainStep::Continue(next_cluster))
    }

    pub(super) fn link_prepared_chain_to_tail(
        &mut self,
        tail_cluster: u32,
        appended_cluster: u32,
    ) -> Result<Result<()>> {
        if !self.boot_region.is_valid_cluster(appended_cluster) {
            return Err(invalid_operation_input());
        }

        match self.next_cluster(tail_cluster)? {
            FatChainStep::End => Ok(self.write_cluster_entry(tail_cluster, appended_cluster)),
            FatChainStep::Continue(_) => Err(invalid_on_disk_layout()),
        }
    }

    pub(super) fn link_contiguous_chain_to_prepared_cluster(
        &mut self,
        start_cluster: u32,
        cluster_count: usize,
        appended_cluster: u32,
    ) -> Result<()> {
        if cluster_count == 0 || !self.boot_region.is_valid_cluster(appended_cluster) {
            return Err(invalid_operation_input());
        }

        for cluster_offset in (0..cluster_count).rev() {
            let current_cluster = start_cluster
                .checked_add(u32::try_from(cluster_offset).map_err(|_| invalid_operation_input())?)
                .ok_or_else(invalid_operation_input)?;
            if !self.boot_region.is_valid_cluster(current_cluster) {
                return Err(invalid_operation_input());
            }

            let next_cluster = if cluster_offset + 1 == cluster_count {
                appended_cluster
            } else {
                current_cluster
                    .checked_add(1)
                    .ok_or_else(invalid_operation_input)?
            };
            self.write_cluster_entry(current_cluster, next_cluster)?;
        }
        Ok(())
    }

    pub(super) fn link_contiguous_chain_to_cluster(
        &mut self,
        start_cluster: u32,
        cluster_count: usize,
        appended_cluster: u32,
    ) -> Result<()> {
        if cluster_count == 0 || !self.boot_region.is_valid_cluster(appended_cluster) {
            return Err(invalid_operation_input());
        }

        self.write_cluster_entry(appended_cluster, FAT_END_OF_CHAIN)?;
        self.link_contiguous_chain_to_prepared_cluster(
            start_cluster,
            cluster_count,
            appended_cluster,
        )
    }

    pub(super) fn terminate_cluster_chain(&mut self, cluster: u32) -> Result<()> {
        self.write_cluster_entry(cluster, FAT_END_OF_CHAIN)
    }

    fn write_cluster_entry(&mut self, cluster: u32, next_cluster: u32) -> Result<()> {
        if !self.boot_region.is_valid_cluster(cluster) {
            return Err(invalid_operation_input());
        }

        let (sector_index, entry_range) = self.cached_entry_range_for_cluster(cluster)?;
        self.cached_sector
            .get_mut(entry_range)
            .ok_or_else(invalid_on_disk_layout)?
            .copy_from_slice(&next_cluster.to_le_bytes());

        let sector_size =
            u64::try_from(self.boot_region.sector_size).map_err(|_| invalid_on_disk_layout())?;
        let sector_offset = sector_index
            .checked_mul(sector_size)
            .ok_or_else(invalid_on_disk_layout)?;
        self.block_device
            .write_bytes(
                usize::try_from(sector_offset).map_err(|_| invalid_on_disk_layout())?,
                &self.cached_sector,
            )
            .map_err(|_| device_io())
    }

    fn cached_entry_range_for_cluster(&mut self, cluster: u32) -> Result<(u64, Range<usize>)> {
        let sector_size =
            u64::try_from(self.boot_region.sector_size).map_err(|_| invalid_on_disk_layout())?;
        let fat_start = u64::from(self.boot_region.fat_offset_sectors)
            .checked_mul(sector_size)
            .ok_or_else(invalid_on_disk_layout)?;
        let fat_end = u64::from(self.boot_region.fat_length_sectors)
            .checked_mul(sector_size)
            .and_then(|fat_bytes| fat_start.checked_add(fat_bytes))
            .ok_or_else(invalid_on_disk_layout)?;
        let entry_offset = u64::from(cluster)
            .checked_mul(FAT_ENTRY_SIZE)
            .and_then(|cluster_offset| fat_start.checked_add(cluster_offset))
            .ok_or_else(invalid_on_disk_layout)?;
        let entry_end_offset = entry_offset
            .checked_add(FAT_ENTRY_SIZE)
            .ok_or_else(invalid_on_disk_layout)?;
        if entry_end_offset > fat_end {
            return Err(invalid_on_disk_layout());
        }
        let sector_index = entry_offset / sector_size;
        if self.cached_sector_index != Some(sector_index) {
            let sector_offset = sector_index
                .checked_mul(sector_size)
                .ok_or_else(invalid_on_disk_layout)?;
            self.block_device
                .read_bytes(
                    usize::try_from(sector_offset).map_err(|_| invalid_on_disk_layout())?,
                    &mut self.cached_sector,
                )
                .map_err(|_| device_io())?;
            self.cached_sector_index = Some(sector_index);
        }
        let entry_within_sector =
            usize::try_from(entry_offset % sector_size).map_err(|_| invalid_on_disk_layout())?;
        let entry_end = entry_within_sector
            .checked_add(size_of::<u32>())
            .ok_or_else(invalid_on_disk_layout)?;
        Ok((sector_index, entry_within_sector..entry_end))
    }
}
