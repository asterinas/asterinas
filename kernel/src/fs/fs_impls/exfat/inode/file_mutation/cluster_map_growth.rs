// SPDX-License-Identifier: MPL-2.0

//! Owns regular-file cluster-map growth shape and prepared FAT range linking.
//!
//! This child module plans how a regular-file cluster map grows
//! once allocation has produced new cluster ranges.
//! It decides whether the grown file can stay contiguous,
//! when FAT links must be materialized,
//! and how a new validated cluster-map generation is assembled before publication.
//!
//! Its entry points derive the grown mapping topology
//! and prepare the FAT updates needed to splice newly allocated ranges into the file.
//! The data model is the old validated cluster map plus the candidate allocated ranges
//! measured against the mounted boot geometry.
//!
//! Locking and recovery matter because FAT publication must stay coordinated
//! with allocation accounting and later inode/page-cache publication.
//! A failed growth step must not leave the caller unable to distinguish
//! pre-publication rollback from already-published state that requires stronger recovery.
//!
//! This module is limited to cluster-map growth topology.
//! It does not own user-visible write dispatch,
//! page-cache dirtying,
//! or entry-set persistence.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 4, 5.1, 7.6.6, and 8.1.

use super::super::{
    super::{
        bitmap::ClusterRange, boot::BootRegion, fat::FatReader, inconsistent_bitmap_accounting,
        invalid_on_disk_layout, invalid_operation_input,
    },
    ClusterMap, ExfatInode, StreamExtensionDirEntry,
};
use crate::prelude::*;

impl ExfatInode {
    pub(super) fn grow_cluster_map(
        boot_region: &BootRegion,
        cluster_map_generation: &ClusterMap,
        new_data_length: usize,
        allocated_ranges: &[ClusterRange],
    ) -> Result<ClusterMap> {
        let cluster_map = cluster_map_generation.stream_extension();
        let Some(current_data_length) = cluster_map.data_length else {
            return_errno!(Errno::EINVAL);
        };
        let Some(current_valid_data_length) = cluster_map.valid_data_length else {
            return_errno!(Errno::EINVAL);
        };
        if current_valid_data_length > current_data_length || new_data_length < current_data_length
        {
            return_errno!(Errno::EINVAL);
        }
        if new_data_length == current_data_length {
            return ClusterMap::from_stream_and_ranges(
                boot_region,
                cluster_map,
                cluster_map_generation.cluster_ranges().to_vec(),
            );
        }

        let current_allocated_clusters = if current_data_length == 0 {
            0
        } else {
            current_data_length.div_ceil(boot_region.cluster_size)
        };
        let target_allocated_clusters = new_data_length.div_ceil(boot_region.cluster_size);
        if target_allocated_clusters == current_allocated_clusters {
            return ClusterMap::from_stream_and_ranges(
                boot_region,
                StreamExtensionDirEntry {
                    data_length: Some(new_data_length),
                    ..cluster_map
                },
                cluster_map_generation.cluster_ranges().to_vec(),
            );
        }

        let additional_clusters = target_allocated_clusters
            .checked_sub(current_allocated_clusters)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let allocated_cluster_count =
            allocated_ranges
                .iter()
                .try_fold(0usize, |total_clusters, range| {
                    total_clusters
                        .checked_add(range.cluster_count)
                        .ok_or_else(inconsistent_bitmap_accounting)
                })?;
        if allocated_cluster_count != additional_clusters {
            return Err(inconsistent_bitmap_accounting());
        }
        if current_allocated_clusters == 0 {
            return Self::allocate_initial_regular_file_clusters(
                boot_region,
                cluster_map,
                new_data_length,
                allocated_ranges,
            );
        }

        let first_new_cluster = allocated_ranges
            .first()
            .ok_or_else(inconsistent_bitmap_accounting)?
            .start_cluster;
        if cluster_map.no_fat_chain
            && allocated_ranges.len() == 1
            && cluster_map.first_cluster.checked_add(
                u32::try_from(current_allocated_clusters).map_err(|_| invalid_on_disk_layout())?,
            ) == Some(first_new_cluster)
        {
            return cluster_map_generation.appended(
                boot_region,
                Self::extend_contiguous_regular_file_clusters(cluster_map, new_data_length),
                allocated_ranges,
            );
        }

        Self::extend_fragmented_regular_file_clusters(
            boot_region,
            cluster_map_generation,
            new_data_length,
            allocated_ranges,
        )
    }

    fn allocate_initial_regular_file_clusters(
        boot_region: &BootRegion,
        cluster_map: StreamExtensionDirEntry,
        new_data_length: usize,
        allocated_ranges: &[ClusterRange],
    ) -> Result<ClusterMap> {
        let first_new_cluster = allocated_ranges
            .first()
            .ok_or_else(inconsistent_bitmap_accounting)?
            .start_cluster;
        let is_single_contiguous_allocation = allocated_ranges.len() == 1;
        ClusterMap::from_stream_and_ranges(
            boot_region,
            StreamExtensionDirEntry {
                data_length: Some(new_data_length),
                first_cluster: first_new_cluster,
                no_fat_chain: is_single_contiguous_allocation,
                ..cluster_map
            },
            allocated_ranges.to_vec(),
        )
    }

    fn extend_contiguous_regular_file_clusters(
        cluster_map: StreamExtensionDirEntry,
        new_data_length: usize,
    ) -> StreamExtensionDirEntry {
        StreamExtensionDirEntry {
            data_length: Some(new_data_length),
            ..cluster_map
        }
    }

    fn extend_fragmented_regular_file_clusters(
        boot_region: &BootRegion,
        cluster_map_generation: &ClusterMap,
        new_data_length: usize,
        allocated_ranges: &[ClusterRange],
    ) -> Result<ClusterMap> {
        let cluster_map = cluster_map_generation.stream_extension();
        cluster_map_generation.appended(
            boot_region,
            StreamExtensionDirEntry {
                data_length: Some(new_data_length),
                no_fat_chain: false,
                ..cluster_map
            },
            allocated_ranges,
        )
    }

    pub(super) fn link_allocated_cluster_ranges(
        fat_reader: &mut FatReader<'_>,
        allocated_ranges: &[ClusterRange],
    ) -> Result<()> {
        for (range_index, range) in allocated_ranges.iter().enumerate() {
            let next_range_start = allocated_ranges
                .get(range_index + 1)
                .map(|next_range| next_range.start_cluster);
            match (range.cluster_count, next_range_start) {
                (0, _) => return Err(invalid_operation_input()),
                (1, None) => fat_reader.terminate_cluster_chain(range.start_cluster)?,
                (cluster_count, None) => {
                    let last_cluster = range
                        .start_cluster
                        .checked_add(
                            u32::try_from(cluster_count - 1)
                                .map_err(|_| invalid_operation_input())?,
                        )
                        .ok_or_else(invalid_operation_input)?;
                    fat_reader.link_contiguous_chain_to_cluster(
                        range.start_cluster,
                        cluster_count - 1,
                        last_cluster,
                    )?;
                }
                (cluster_count, Some(next_cluster)) => {
                    fat_reader.link_contiguous_chain_to_cluster(
                        range.start_cluster,
                        cluster_count,
                        next_cluster,
                    )?;
                }
            }
        }
        Ok(())
    }
}
