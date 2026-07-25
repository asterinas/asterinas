// SPDX-License-Identifier: MPL-2.0

//! Owns replaced-target cleanup and retired namespace inode detachment helpers.
//!
//! This child module handles the namespace participants that stop being live
//! after rename or removal commits.
//! It collects retired cluster ranges,
//! cleans up replaced targets,
//! and detaches or retires inodes whose directory presence has been removed.
//!
//! Its entry points cover retired mapping collection,
//! replaced-target cleanup state,
//! and detached-inode retirement helpers used by the parent mutation flow.
//! The data model is the retired inode's validated cluster map and directory-entry relationship
//! after the namespace transition has already been admitted.
//!
//! Lock ordering remains important because cleanup still operates in the shared inode domain,
//! and recovery policy is stricter here because some namespace effects are already committed.
//! Retry, escalation, and forced-shutdown decisions therefore stay explicit
//! instead of being folded into generic deletion logic.
//!
//! This module is limited to post-admission retirement and cleanup.
//! It does not own rename discovery,
//! slot mutation,
//! or the initial namespace decision.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 5.1, 7.4, 7.6, and 8.1.

use aster_block::BlockDevice;

use super::{
    super::{ClusterMap, ExfatInode, StreamExtensionDirEntry, state::InodeStateWriteGuard},
    admission::AdmittedRenameChild,
};
use crate::{
    fs::{
        exfat::{
            bitmap::{AllocGuard, ClusterRange},
            boot::BootRegion,
            dir_entry_format::{DirEntrySlotRange, FileEntrySetView},
            fat::{ChainVisitControl, FatReader},
            fs::{ExfatFs, FsState},
            invalid_on_disk_layout,
        },
        file::InodeType,
    },
    prelude::*,
};

pub(super) enum ReplacedTargetCleanup {
    Immediate {
        slot_range: DirEntrySlotRange,
        ranges: Vec<ClusterRange>,
    },
    CachedGeneration {
        slot_range: DirEntrySlotRange,
        cluster_map: Arc<ClusterMap>,
        ranges: Vec<ClusterRange>,
    },
}

#[derive(Clone, Copy)]
pub(super) enum RenameTargetRemovalState {
    Persisted,
    Uncertain,
}

impl ExfatInode {
    pub(super) fn allocated_cluster_ranges(
        block_device: &Arc<dyn BlockDevice>,
        boot_region: &BootRegion,
        stream_entry: StreamExtensionDirEntry,
    ) -> Result<Vec<ClusterRange>> {
        let data_length = stream_entry
            .data_length
            .ok_or_else(invalid_on_disk_layout)?;
        let first_cluster = stream_entry.first_cluster;
        if data_length == 0 {
            if first_cluster != 0 {
                return Err(invalid_on_disk_layout());
            }
            return Ok(Vec::new());
        }

        boot_region.validate_stream_data(
            first_cluster,
            u64::try_from(data_length).map_err(|_| invalid_on_disk_layout())?,
        )?;
        let expected_cluster_count = data_length.div_ceil(boot_region.cluster_size);
        if stream_entry.no_fat_chain {
            return Ok(vec![ClusterRange {
                start_cluster: first_cluster,
                cluster_count: expected_cluster_count,
            }]);
        }

        let mut cluster_ranges = Vec::new();
        let mut current_range_start = 0u32;
        let mut current_range_count = 0usize;
        let mut previous_cluster: Option<u32> = None;
        let mut total_cluster_count = 0usize;
        let mut fat_reader = FatReader::new(block_device.as_ref(), boot_region);
        fat_reader.walk_cluster_chain(first_cluster, |cluster, _| {
            total_cluster_count = total_cluster_count
                .checked_add(1)
                .ok_or(invalid_on_disk_layout())?;
            match previous_cluster {
                Some(previous_cluster) if previous_cluster.checked_add(1) == Some(cluster) => {
                    current_range_count = current_range_count
                        .checked_add(1)
                        .ok_or(invalid_on_disk_layout())?;
                }
                Some(_) => {
                    cluster_ranges.push(ClusterRange {
                        start_cluster: current_range_start,
                        cluster_count: current_range_count,
                    });
                    current_range_start = cluster;
                    current_range_count = 1;
                }
                None => {
                    current_range_start = cluster;
                    current_range_count = 1;
                }
            }
            previous_cluster = Some(cluster);
            Ok(ChainVisitControl::Continue)
        })?;
        if current_range_count == 0 || total_cluster_count != expected_cluster_count {
            return Err(invalid_on_disk_layout());
        }
        cluster_ranges.push(ClusterRange {
            start_cluster: current_range_start,
            cluster_count: current_range_count,
        });
        Ok(cluster_ranges)
    }

    pub(super) fn collect_replaced_target_cleanup(
        target_view: Option<FileEntrySetView<'_>>,
        target_child_inode: Option<&Arc<Self>>,
        target_child_inode_state_guard: Option<&InodeStateWriteGuard<'_>>,
        source_inode_type: InodeType,
        block_device: &Arc<dyn BlockDevice>,
        boot_region: &BootRegion,
        allocation_guard: &AllocGuard<'_>,
    ) -> Result<Option<ReplacedTargetCleanup>> {
        let Some(target_view) = target_view else {
            return Ok(None);
        };
        let target_slot_range = target_view.slot_range();
        let (target_inode_type, _, _, _) = target_view.child_metadata(boot_region)?;
        if source_inode_type == InodeType::Dir && target_inode_type != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if source_inode_type != InodeType::Dir && target_inode_type == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        if target_inode_type == InodeType::Dir {
            let (Some(child_inode), Some(child_inode_state_guard)) =
                (target_child_inode, target_child_inode_state_guard)
            else {
                return Err(invalid_on_disk_layout());
            };
            Self::ensure_directory_snapshot_is_empty(
                child_inode.as_ref(),
                child_inode_state_guard,
                allocation_guard,
                boot_region,
            )?;
        }
        if target_inode_type == InodeType::File
            && let (Some(child_inode), Some(child_inode_state_guard)) =
                (target_child_inode, target_child_inode_state_guard)
        {
            let detached_regular_file_reclaim = Self::capture_cached_regular_file_retirement(
                child_inode,
                child_inode_state_guard,
                allocation_guard,
            )?;
            return Ok(Some(ReplacedTargetCleanup::CachedGeneration {
                slot_range: target_slot_range,
                cluster_map: detached_regular_file_reclaim.0,
                ranges: detached_regular_file_reclaim.1,
            }));
        }
        let replaced_target_ranges =
            Self::allocated_cluster_ranges(block_device, boot_region, target_view.cluster_map()?)?;
        Ok(Some(ReplacedTargetCleanup::Immediate {
            slot_range: target_slot_range,
            ranges: replaced_target_ranges,
        }))
    }

    pub(super) fn cleanup_replaced_target_ranges(
        fs_state: &mut FsState,
        allocation_guard: &mut AllocGuard<'_>,
        replaced_target_ranges: &[ClusterRange],
    ) -> Result<()> {
        if replaced_target_ranges.is_empty() {
            return Ok(());
        }
        allocation_guard.free_clusters(replaced_target_ranges)?;
        ExfatFs::disable_unsupported_discard_after_release(fs_state);
        Ok(())
    }

    pub(super) fn capture_cached_regular_file_retirement(
        child_inode: &Arc<Self>,
        child_inode_state_guard: &InodeStateWriteGuard<'_>,
        allocation_guard: &AllocGuard<'_>,
    ) -> Result<(Arc<ClusterMap>, Vec<ClusterRange>)> {
        let retired_generation =
            child_inode.ensure_cluster_map(child_inode_state_guard, allocation_guard)?;
        let retired_ranges = retired_generation.cluster_ranges().to_vec();
        Ok((retired_generation, retired_ranges))
    }

    pub(super) fn detach_namespace_removed_inode(
        fs_state: &mut FsState,
        allocation_guard: &mut AllocGuard<'_>,
        child_ino: u64,
        child_inode: &Arc<Self>,
        child_inode_state_guard: &InodeStateWriteGuard<'_>,
        detached_regular_file_reclaim: Option<(Arc<ClusterMap>, Vec<ClusterRange>)>,
    ) -> Result<()> {
        child_inode_state_guard.set_parent(Weak::new());
        child_inode.clear_entry_set_location_hint();
        child_inode_state_guard.with_metadata_mut(|metadata| metadata.nr_hard_links = 0);
        if let Some((retired_generation, retired_ranges)) = detached_regular_file_reclaim {
            child_inode
                .clear_detached_regular_file_publish_debt_with_guard(child_inode_state_guard);
            ExfatFs::remove_cached_inode(fs_state, child_ino);
            allocation_guard.lazy_reclaim_clusters(retired_generation, retired_ranges)?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Rename finalization must receive the admitted source and optional target participants plus shared cleanup state explicitly to preserve post-persistence error and cleanup gating."
    )]
    pub(super) fn finalize_rename_protocol(
        destination_directory: &ExfatInode,
        destination_slot_range: DirEntrySlotRange,
        new_source_ino: u64,
        source_child: AdmittedRenameChild<'_, '_>,
        target_child: Option<AdmittedRenameChild<'_, '_>>,
        replacement: Option<ReplacedTargetCleanup>,
        fs_state: &mut FsState,
        allocation_guard: &mut AllocGuard<'_>,
        rename_target_removal_state: RenameTargetRemovalState,
    ) -> Result<()> {
        let old_source_ino = source_child.guard.metadata().ino;
        let replaced_target_ino = target_child
            .as_ref()
            .map(|child| child.guard.metadata().ino);
        source_child
            .guard
            .set_parent(destination_directory.weak_self());
        source_child
            .guard
            .with_metadata_mut(|metadata| metadata.ino = new_source_ino);
        if source_child.guard.metadata().type_ == InodeType::File {
            source_child
                .inode
                .store_entry_set_location_hint(destination_slot_range)?;
        }
        let mut finalization_error = None;
        let (replaced_target_ranges, detached_regular_file_reclaim) = match replacement {
            Some(ReplacedTargetCleanup::Immediate { ranges, .. }) => (ranges, None),
            Some(ReplacedTargetCleanup::CachedGeneration {
                cluster_map,
                ranges,
                ..
            }) => (Vec::new(), Some((cluster_map, ranges))),
            None => (Vec::new(), None),
        };
        if let Some(target_child) = target_child
            && let Err(error) = Self::detach_namespace_removed_inode(
                fs_state,
                allocation_guard,
                target_child.guard.metadata().ino,
                target_child.inode,
                target_child.guard,
                match rename_target_removal_state {
                    RenameTargetRemovalState::Persisted => detached_regular_file_reclaim,
                    RenameTargetRemovalState::Uncertain => None,
                },
            )
            && finalization_error.is_none()
        {
            finalization_error = Some(error);
        }
        ExfatFs::rebind_rename_inode_cache(
            fs_state,
            old_source_ino,
            new_source_ino,
            source_child.inode,
            replaced_target_ino,
        );
        if matches!(
            rename_target_removal_state,
            RenameTargetRemovalState::Persisted
        ) && finalization_error.is_none()
            && let Err(error) = Self::cleanup_replaced_target_ranges(
                fs_state,
                allocation_guard,
                &replaced_target_ranges,
            )
        {
            finalization_error = Some(error);
        }
        if let Some(error) = finalization_error {
            return Err(error);
        }
        Ok(())
    }
}
