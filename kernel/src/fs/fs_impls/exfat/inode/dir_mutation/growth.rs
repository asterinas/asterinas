// SPDX-License-Identifier: MPL-2.0

//! Owns directory cluster-map growth and publication helpers.
//!
//! This child module extends directory storage when namespace mutation runs out of slots.
//! It plans new directory cluster ranges,
//! attaches them to the directory stream,
//! and publishes the grown cluster-map state once allocation and entry-set updates agree.
//!
//! Its entry points cover directory cluster-map growth,
//! directory attachment to parent-visible state,
//! and the publication helpers used after growth succeeds.
//! The data model is the directory cluster map plus the allocated ranges and entry-set bytes
//! needed to make the growth visible.
//!
//! Locking and allocation ordering matter because directory growth touches inode state,
//! allocation bitmap/FAT state,
//! and later parent-entry persistence.
//! Recovery paths preserve rollback before publication where possible
//! and surface stronger failure when a grown directory image can no longer be restored safely.
//!
//! This module is limited to directory growth topology and publication.
//! It does not own rename admission or slot search,
//! and it assumes the outer mutation path has already selected the growth point.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 4, 5.1, 6, 7.4, 7.6, and 8.1.

use aster_block::BlockDevice;
use ostd::mm::VmIo;

use super::super::{
    ClusterMap, ExfatInode, PersistenceRecovery, StreamExtensionDirEntry,
    parent_entry_set::PreparedEntrySetWrite, state::InodeStateWriteGuard,
};
use crate::{
    fs::{
        exfat::{
            bitmap::{AllocGuard, ClusterRange},
            boot::BootRegion,
            device_io,
            fat::FatReader,
            fs::{ExfatFs, FsState},
            invalid_on_disk_layout,
        },
        file::InodeType,
    },
    prelude::*,
};

impl ExfatInode {
    fn initialize_directory_cluster(
        block_device: &Arc<dyn BlockDevice>,
        boot_region: &BootRegion,
        first_cluster: u32,
    ) -> Result<()> {
        let cluster_offset = boot_region.cluster_offset(first_cluster)?;
        let cluster_bytes = vec![0; boot_region.cluster_size];
        block_device
            .write_bytes(cluster_offset, &cluster_bytes)
            .map_err(|_| device_io())
    }

    pub(super) fn prepare_created_child_backing_state(
        &self,
        type_: InodeType,
        zero_size_dir: bool,
        allocation_guard: &mut AllocGuard<'_>,
        fs_state: &mut FsState,
        fs: &ExfatFs,
    ) -> Result<(
        StreamExtensionDirEntry,
        Option<Arc<ClusterMap>>,
        Option<u32>,
    )> {
        let block_device = fs.immutable_block_device();
        let boot_region = fs.immutable_boot_region();
        if type_ == InodeType::Dir && !zero_size_dir {
            allocation_guard.allocate(1, None)?;
            let allocated_cluster = allocation_guard.single_cluster()?;
            if let Err(error) =
                Self::initialize_directory_cluster(&block_device, &boot_region, allocated_cluster)
            {
                if allocation_guard.rollback_allocation()? {
                    ExfatFs::disable_unsupported_discard_after_release(fs_state);
                }
                return Err(error);
            }
            let child_stream = StreamExtensionDirEntry {
                data_length: Some(boot_region.cluster_size),
                first_cluster: allocated_cluster,
                valid_data_length: Some(boot_region.cluster_size),
                no_fat_chain: true,
            };
            let child_cluster_map = Some(Arc::new(ClusterMap::from_stream_and_ranges(
                &boot_region,
                child_stream,
                vec![ClusterRange {
                    start_cluster: allocated_cluster,
                    cluster_count: 1,
                }],
            )?));
            Ok((child_stream, child_cluster_map, Some(allocated_cluster)))
        } else {
            let child_stream = StreamExtensionDirEntry {
                data_length: Some(0),
                first_cluster: 0,
                valid_data_length: Some(0),
                no_fat_chain: false,
            };
            let child_cluster_map = if type_ == InodeType::Dir {
                Some(Arc::new(ClusterMap::from_stream_and_ranges(
                    &boot_region,
                    child_stream,
                    Vec::new(),
                )?))
            } else {
                None
            };
            Ok((child_stream, child_cluster_map, None))
        }
    }

    pub(super) fn grow_directory_cluster_map(
        &self,
        cluster_map: StreamExtensionDirEntry,
        allocation_guard: &mut AllocGuard<'_>,
        fs_state: &mut FsState,
        fs: &ExfatFs,
        parent_inode_state_guard: Option<&InodeStateWriteGuard<'_>>,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
    ) -> Result<StreamExtensionDirEntry> {
        let block_device = fs.immutable_block_device();
        let boot_region = fs.immutable_boot_region();
        allocation_guard.allocate(1, None)?;
        let allocated_cluster = allocation_guard.single_cluster()?;
        let mut publication_complete = false;
        let update_result = (|| {
            Self::initialize_directory_cluster(&block_device, &boot_region, allocated_cluster)?;
            let (
                updated_cluster_map,
                updated_cluster_map_generation,
                updated_allocated_size,
                exposed_old_topology,
                exposure_error,
                prepared_parent_entry_set_write,
            ) = self.attach_directory_cluster(
                cluster_map,
                allocation_guard,
                self_inode_state_guard,
                fs,
                allocated_cluster,
                |updated_cluster_map| {
                    if updated_cluster_map.data_length.is_none() {
                        return Ok(None);
                    }
                    let parent_inode_state_guard = parent_inode_state_guard.ok_or_else(|| {
                        Error::with_message(
                            Errno::EINVAL,
                            "ordinary exFAT directory growth requires parent write-guard proof",
                        )
                    })?;
                    self.prepare_rewritten_entry_set_write_with_guard(
                        self_inode_state_guard,
                        parent_inode_state_guard,
                        &boot_region,
                        |entry_view| {
                            let (inode_type, _first_cluster, _data_length, _no_fat_chain) =
                                entry_view.child_metadata(&boot_region)?;
                            if inode_type != InodeType::Dir || !entry_view.is_directory() {
                                return Err(invalid_on_disk_layout());
                            }

                            let mut updated_entry_set = entry_view.to_mutable();
                            updated_entry_set.set_cluster_map(&updated_cluster_map)?;
                            Ok(Some(updated_entry_set.into_bytes()))
                        },
                    )
                },
            )?;
            if exposed_old_topology {
                self.commit_directory_cluster_map(
                    self_inode_state_guard,
                    updated_cluster_map_generation.clone(),
                    updated_allocated_size,
                )?;
                allocation_guard.commit_allocation();
                publication_complete = true;
                if let Some(error) = exposure_error {
                    return Err(error);
                }
            }
            let parent_entry_set_write_result =
                if let Some(prepared_parent_entry_set_write) = prepared_parent_entry_set_write {
                    let parent_inode_state_guard = parent_inode_state_guard.ok_or_else(|| {
                        Error::with_message(
                            Errno::EINVAL,
                            "ordinary exFAT directory growth requires parent write-guard proof",
                        )
                    })?;
                    let parent_inode = self_inode_state_guard.parent().ok_or_else(|| {
                        Error::with_message(
                            Errno::EIO,
                            "ordinary exFAT directory parent is not mounted",
                        )
                    })?;
                    if !parent_inode_state_guard.guards_inode(parent_inode.as_ref()) {
                        return Err(Error::new(Errno::EINVAL));
                    }
                    let entry_set_write_result = self.persist_prepared_entry_set_write_classified(
                        fs_state,
                        prepared_parent_entry_set_write,
                        parent_inode.as_ref(),
                        parent_inode_state_guard.metadata(),
                        PersistenceRecovery::RollbackAllowed,
                    )?;
                    Some(entry_set_write_result)
                } else {
                    None
                };
            if !exposed_old_topology
                && matches!(parent_entry_set_write_result.as_ref(), Some(Ok(false)))
            {
                return Err(invalid_on_disk_layout());
            }
            if !exposed_old_topology {
                self.commit_directory_cluster_map(
                    self_inode_state_guard,
                    updated_cluster_map_generation,
                    updated_allocated_size,
                )?;
                allocation_guard.commit_allocation();
                publication_complete = true;
            }
            if let Some(entry_set_write_result) = parent_entry_set_write_result
                && !entry_set_write_result?
            {
                return Err(invalid_on_disk_layout());
            }
            Ok(updated_cluster_map)
        })();
        match update_result {
            Ok(updated_cluster_map) => {
                allocation_guard.commit_allocation();
                Ok(updated_cluster_map)
            }
            Err(error) => {
                if !publication_complete && allocation_guard.rollback_allocation()? {
                    ExfatFs::disable_unsupported_discard_after_release(fs_state);
                }
                Err(error)
            }
        }
    }

    #[expect(
        clippy::type_complexity,
        reason = "The return tuple carries publication state, exposure error, and the opaque prepared parent-entry write across the attach/persist boundary without introducing another carrier."
    )]
    fn attach_directory_cluster(
        &self,
        cluster_map: StreamExtensionDirEntry,
        allocation_guard: &AllocGuard<'_>,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        fs: &ExfatFs,
        allocated_cluster: u32,
        prepare_parent_entry_set_write_fn: impl FnOnce(
            StreamExtensionDirEntry,
        ) -> Result<Option<PreparedEntrySetWrite>>,
    ) -> Result<(
        StreamExtensionDirEntry,
        Arc<ClusterMap>,
        usize,
        bool,
        Option<Error>,
        Option<PreparedEntrySetWrite>,
    )> {
        let block_device = fs.immutable_block_device();
        let boot_region = fs.immutable_boot_region();
        let next_data_length = match cluster_map.data_length {
            Some(data_length) => data_length
                .checked_add(boot_region.cluster_size)
                .ok_or(invalid_on_disk_layout())?,
            None => boot_region.cluster_size,
        };

        let admitted_cluster_map = match cluster_map.data_length {
            Some(_) => self.cluster_map_for_write_guard(
                self_inode_state_guard,
                allocation_guard,
                cluster_map,
            ),
            None => self_inode_state_guard
                .cached_cluster_map()
                .filter(|generation| generation.stream_extension() == cluster_map)
                .ok_or_else(invalid_on_disk_layout),
        }?;
        if self_inode_state_guard.dir_entry_stream() != cluster_map {
            return Err(invalid_on_disk_layout());
        }
        if cluster_map.data_length.is_none()
            && !self_inode_state_guard
                .cached_cluster_map()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &admitted_cluster_map))
        {
            return Err(invalid_on_disk_layout());
        }

        let updated_cluster_map = match cluster_map.data_length {
            Some(0) => StreamExtensionDirEntry {
                data_length: Some(next_data_length),
                first_cluster: allocated_cluster,
                valid_data_length: Some(next_data_length),
                no_fat_chain: false,
            },
            Some(_) if cluster_map.no_fat_chain => StreamExtensionDirEntry {
                data_length: Some(next_data_length),
                valid_data_length: Some(next_data_length),
                no_fat_chain: false,
                ..cluster_map
            },
            Some(_) => StreamExtensionDirEntry {
                data_length: Some(next_data_length),
                valid_data_length: Some(next_data_length),
                ..cluster_map
            },
            None => cluster_map,
        };
        let updated_generation = Arc::new(admitted_cluster_map.appended(
            &boot_region,
            updated_cluster_map,
            &[ClusterRange {
                start_cluster: allocated_cluster,
                cluster_count: 1,
            }],
        )?);
        let updated_allocated_size = updated_generation.allocated_byte_length(&boot_region)?;
        let exposed_old_topology = match cluster_map.data_length {
            None => true,
            Some(data_length) => data_length != 0 && !cluster_map.no_fat_chain,
        };
        let prepared_parent_entry_set_write =
            prepare_parent_entry_set_write_fn(updated_cluster_map)?;

        let mut fat_reader = FatReader::new(block_device.as_ref(), &boot_region);
        let exposure_error = match cluster_map.data_length {
            Some(0) => {
                fat_reader.terminate_cluster_chain(allocated_cluster)?;
                None
            }
            Some(data_length) if cluster_map.no_fat_chain => {
                let cluster_count = data_length.div_ceil(boot_region.cluster_size);
                fat_reader.link_contiguous_chain_to_cluster(
                    cluster_map.first_cluster,
                    cluster_count,
                    allocated_cluster,
                )?;
                None
            }
            Some(_) | None => {
                fat_reader.terminate_cluster_chain(allocated_cluster)?;
                let tail_cluster = admitted_cluster_map
                    .terminal_cluster(&boot_region)?
                    .ok_or_else(invalid_on_disk_layout)?;
                fat_reader
                    .link_prepared_chain_to_tail(tail_cluster, allocated_cluster)?
                    .err()
            }
        };
        Ok((
            updated_cluster_map,
            updated_generation,
            updated_allocated_size,
            exposed_old_topology,
            exposure_error,
            prepared_parent_entry_set_write,
        ))
    }

    fn commit_directory_cluster_map(
        &self,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        updated_cluster_map_generation: Arc<ClusterMap>,
        updated_allocated_size: usize,
    ) -> Result<()> {
        let metadata = self_inode_state_guard.metadata();
        let previous_size = metadata.size;
        let page_cache_context = self.page_cache_context_for_mapping(
            metadata,
            updated_cluster_map_generation.clone(),
            updated_allocated_size,
            updated_allocated_size,
        )?;
        let _ = self_inode_state_guard
            .replace_dir_entry_stream(updated_cluster_map_generation.stream_extension());
        self_inode_state_guard.set_cached_cluster_map(updated_cluster_map_generation);
        let _ = self_inode_state_guard.replace_page_cache_context(page_cache_context);
        self_inode_state_guard.with_metadata_mut(|metadata| {
            metadata.size = updated_allocated_size;
        });
        if let Some(page_cache) = self
            .page_cache
            .get()
            .and_then(|page_cache| page_cache.as_ref())
        {
            page_cache.resize(updated_allocated_size, previous_size)?;
        }
        Ok(())
    }
}
