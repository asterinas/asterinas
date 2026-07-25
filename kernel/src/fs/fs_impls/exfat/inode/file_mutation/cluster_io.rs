// SPDX-License-Identifier: MPL-2.0

//! Owns regular-file mapping validation, cluster lookup, and direct cluster mutation.
//!
//! This child module owns the lowest-level regular-file cluster I/O helpers
//! used by the write path after higher-level admission and growth planning.
//! It validates that a regular-file mapping shape matches boot geometry,
//! locates the cluster covering a file offset,
//! and performs direct cluster reads or writes against the backing device.
//!
//! The data model is the validated cluster map plus boot-region geometry
//! translated into cluster indices and byte ranges.
//! Allocation and FAT guard assumptions come from the caller,
//! so this module can focus on mapping correctness and cluster-level mutation.
//!
//! Partial-failure handling is conservative:
//! invalid mapping state is rejected before I/O,
//! and direct cluster mutation reports device or topology failure
//! without publishing higher-level metadata changes by itself.
//!
//! This module is limited to cluster-local regular-file I/O.
//! It does not own page-cache publication,
//! namespace recovery,
//! or stream-length updates.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 5.1, 7.6.6, 7.6.7, and 8.1,
//! plus `aster_block::BlockDevice`.

use aster_block::BlockDevice;
use ostd::mm::VmIo;

use super::super::{super::boot::BootRegion, ClusterMap, ExfatInode, StreamExtensionDirEntry};
use crate::prelude::*;

impl ExfatInode {
    fn validate_regular_file_mapping_shape(
        boot_region: &BootRegion,
        cluster_map: &StreamExtensionDirEntry,
        data_length: usize,
    ) -> Result<()> {
        let data_length_u64 = u64::try_from(data_length).map_err(|_| Error::new(Errno::EINVAL))?;
        match boot_region.validate_stream_data(cluster_map.first_cluster, data_length_u64) {
            Ok(()) => Ok(()),
            Err(_) => return_errno!(Errno::EINVAL),
        }
    }

    fn mapped_regular_file_cluster(
        boot_region: &BootRegion,
        cluster_map: &ClusterMap,
        cluster_index: usize,
    ) -> Result<u32> {
        let (data_length, _) = cluster_map.validated_data_lengths()?;
        let stream_extension = cluster_map.stream_extension();
        if stream_extension.no_fat_chain {
            let cluster_count = data_length.div_ceil(boot_region.cluster_size);
            if cluster_index >= cluster_count {
                return_errno!(Errno::EINVAL);
            }
            let last_cluster = stream_extension
                .first_cluster
                .checked_add(
                    u32::try_from(cluster_count.saturating_sub(1))
                        .map_err(|_| Error::new(Errno::EINVAL))?,
                )
                .ok_or_else(|| Error::new(Errno::EINVAL))?;
            if !boot_region.is_valid_cluster(last_cluster) {
                return_errno!(Errno::EINVAL);
            }
            return stream_extension
                .first_cluster
                .checked_add(u32::try_from(cluster_index).map_err(|_| Error::new(Errno::EINVAL))?)
                .ok_or_else(|| Error::new(Errno::EINVAL));
        }

        cluster_map.mapped_cluster(boot_region, cluster_index)
    }

    pub(super) fn mutate_regular_file_range(
        block_device: &Arc<dyn BlockDevice>,
        boot_region: &BootRegion,
        cluster_map: &ClusterMap,
        offset: usize,
        len: usize,
        mut fill_chunk_fn: impl FnMut(&mut [u8]) -> Result<()>,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }

        let (data_length, _) = cluster_map.validated_data_lengths()?;
        Self::validate_regular_file_mapping_shape(
            boot_region,
            &cluster_map.stream_extension(),
            data_length,
        )?;
        let write_end = offset
            .checked_add(len)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if write_end > data_length {
            return_errno!(Errno::EOPNOTSUPP);
        }

        let cluster_size = boot_region.cluster_size;
        let mut cluster_index = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;
        let mut remaining = len;
        let mut cluster_buffer = vec![0; cluster_size];
        while remaining != 0 {
            let current_cluster =
                Self::mapped_regular_file_cluster(boot_region, cluster_map, cluster_index)?;
            let chunk_len = remaining.min(cluster_size - cluster_offset);
            let chunk_end = cluster_offset
                .checked_add(chunk_len)
                .ok_or_else(|| Error::new(Errno::EINVAL))?;
            let cluster_start = boot_region.cluster_offset(current_cluster)?;
            block_device
                .read_bytes(cluster_start, &mut cluster_buffer)
                .map_err(|_| Error::new(Errno::EIO))?;
            fill_chunk_fn(&mut cluster_buffer[cluster_offset..chunk_end])?;
            block_device
                .write_bytes(cluster_start, &cluster_buffer)
                .map_err(|_| Error::new(Errno::EIO))?;
            remaining -= chunk_len;
            cluster_offset = 0;
            cluster_index += 1;
            if remaining == 0 {
                break;
            }
        }
        Ok(())
    }
}
