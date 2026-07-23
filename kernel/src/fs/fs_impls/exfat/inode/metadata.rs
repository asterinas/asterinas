// SPDX-License-Identifier: MPL-2.0

//! Projects and mutates inode metadata backed by exFAT file-entry sets.
//!
//! This module owns the metadata view derived from exFAT file-entry sets.
//! It projects cached metadata for VFS queries,
//! applies metadata mutations such as timestamps and size-related fields,
//! and rewrites the backing entry set when inode metadata must be persisted.
//!
//! Its entry points cover VFS metadata getters and setters,
//! timestamp conversion,
//! entry-set rewrite helpers,
//! and directory metadata refresh after namespace changes.
//! The data model is the correspondence between in-memory metadata fields
//! and the on-disk file-entry and stream-extension records.
//!
//! Lock ordering and dirty publication are important here
//! because metadata changes may involve parent directories and shared persistence helpers.
//! Recovery paths preserve whether failure happened before or after entry-set rewrite preparation
//! so sync and forced-shutdown policy stay coherent.
//!
//! This module is limited to metadata projection and persistence.
//! It does not own directory admission or cluster allocation policy,
//! and it rejects invalid on-disk metadata encodings rather than synthesizing replacements.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 7.4 and 7.6,
//! plus `crate::fs::vfs::inode::Metadata`.

use core::{cell::Cell, time::Duration};

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use super::{
    super::{
        boot::BootRegion,
        dir_entry_format::{self as direntry, FileEntrySetView, FileEntryTimestamp},
        fs::{ExfatFs, FsState},
        invalid_on_disk_layout,
    },
    ExfatInode, PersistenceRecovery,
    parent_entry_set::PreparedEntrySetWrite,
    state::InodeStateWriteGuard,
};
use crate::{
    fs::{
        file::{InodeMode, InodeType, chmod, mkmod},
        vfs::{file_system::FsFlags, inode::Metadata},
    },
    prelude::*,
    process::{Gid, Uid},
    time::clocks::{RealTimeClock, RealTimeCoarseClock},
};

#[derive(Clone, Copy)]
pub(super) enum InodeTimestampField {
    Accessed,
    Modified,
}

impl ExfatInode {
    pub(super) fn decoded_exfat_timestamp(
        timestamp_bytes: [u8; 4],
        ten_ms_increment: Option<u8>,
        utc_offset_byte: u8,
    ) -> Result<Duration> {
        if timestamp_bytes == [0; 4] && ten_ms_increment.unwrap_or(0) == 0 {
            return Ok(Duration::ZERO);
        }
        let encoded_date = u16::from_le_bytes([timestamp_bytes[2], timestamp_bytes[3]]);
        let encoded_year = 1980i32 + i32::from(encoded_date >> 9);
        let encoded_month =
            u8::try_from((encoded_date >> 5) & 0x0f).map_err(|_| invalid_on_disk_layout())?;
        let encoded_day =
            u8::try_from(encoded_date & 0x1f).map_err(|_| invalid_on_disk_layout())?;
        let month = Month::try_from(encoded_month).map_err(|_| invalid_on_disk_layout())?;
        let date = Date::from_calendar_date(encoded_year, month, encoded_day)
            .map_err(|_| invalid_on_disk_layout())?;
        let encoded_time = u16::from_le_bytes([timestamp_bytes[0], timestamp_bytes[1]]);
        let hour =
            u8::try_from((encoded_time >> 11) & 0x1f).map_err(|_| invalid_on_disk_layout())?;
        let minute =
            u8::try_from((encoded_time >> 5) & 0x3f).map_err(|_| invalid_on_disk_layout())?;
        let mut seconds = u8::try_from(encoded_time & 0x1f)
            .map_err(|_| invalid_on_disk_layout())?
            .checked_mul(2)
            .ok_or_else(invalid_on_disk_layout)?;
        let mut milliseconds = 0u16;
        if let Some(ten_ms_increment) = ten_ms_increment {
            if ten_ms_increment >= 200 {
                return Err(invalid_on_disk_layout());
            }
            seconds = seconds
                .checked_add(ten_ms_increment / 100)
                .ok_or(invalid_on_disk_layout())?;
            milliseconds = u16::from(ten_ms_increment % 100) * 10;
        }
        let time = Time::from_hms_milli(hour, minute, seconds, milliseconds)
            .map_err(|_| invalid_on_disk_layout())?;
        let utc_offset = Self::exfat_utc_offset(utc_offset_byte)?;
        let date_time = PrimitiveDateTime::new(date, time).assume_offset(utc_offset);
        let unix_timestamp_nanos = u64::try_from(date_time.unix_timestamp_nanos())
            .map_err(|_| invalid_on_disk_layout())?;
        Ok(Duration::from_nanos(unix_timestamp_nanos))
    }

    pub(super) fn exfat_utc_offset(utc_offset_byte: u8) -> Result<UtcOffset> {
        if utc_offset_byte & 0x80 == 0 {
            return Ok(UtcOffset::UTC);
        }
        let quarter_hours = (((utc_offset_byte & 0x7f) as i8) << 1) >> 1;
        UtcOffset::from_whole_seconds(i32::from(quarter_hours) * 15 * 60)
            .map_err(|_| invalid_on_disk_layout())
    }

    pub(super) fn encoded_exfat_timestamp_fields(
        timestamp: Duration,
        utc_offset_byte: u8,
    ) -> Result<([u8; 4], u8, u8)> {
        let unix_nanos =
            i128::try_from(timestamp.as_nanos()).map_err(|_| Error::new(Errno::EINVAL))?;
        let utc_offset = Self::exfat_utc_offset(utc_offset_byte)?;
        let date_time = OffsetDateTime::from_unix_timestamp_nanos(unix_nanos)
            .map_err(|_| Error::new(Errno::EINVAL))?
            .to_offset(utc_offset);
        let encoded_utc_offset = if utc_offset_byte & 0x80 == 0 {
            0
        } else {
            utc_offset_byte
        };
        let (
            encoded_year,
            encoded_month,
            encoded_day,
            encoded_hour,
            encoded_minute,
            encoded_second,
            encoded_millisecond,
        ) = match date_time.year() {
            ..1980 => (1980, 1u8, 1u8, 0u8, 0u8, 0u8, 0u16),
            2108.. => (2107, 12u8, 31u8, 23u8, 59u8, 59u8, 990u16),
            year => (
                year,
                date_time.month() as u8,
                date_time.day(),
                date_time.hour(),
                date_time.minute(),
                date_time.second(),
                date_time.millisecond(),
            ),
        };
        let date = ((u16::try_from(encoded_year - 1980).map_err(|_| Error::new(Errno::EINVAL))?)
            << 9)
            | (u16::from(encoded_month) << 5)
            | u16::from(encoded_day);
        let time = (u16::from(encoded_hour) << 11)
            | (u16::from(encoded_minute) << 5)
            | u16::from(encoded_second / 2);
        let date_bytes = date.to_le_bytes();
        let time_bytes = time.to_le_bytes();
        let hundredths_increment = u16::from(encoded_second % 2) * 100 + encoded_millisecond / 10;
        Ok((
            [time_bytes[0], time_bytes[1], date_bytes[0], date_bytes[1]],
            u8::try_from(hundredths_increment).map_err(|_| Error::new(Errno::EINVAL))?,
            encoded_utc_offset,
        ))
    }

    pub(super) fn regular_file_allocated_sectors(
        boot_region: &BootRegion,
        data_length: usize,
    ) -> Result<usize> {
        let allocated_clusters = if data_length == 0 {
            0
        } else {
            data_length.div_ceil(boot_region.cluster_size)
        };
        allocated_clusters
            .checked_mul(boot_region.sectors_per_cluster)
            .ok_or_else(invalid_on_disk_layout)
    }
}

// ---- meta_write (refresh + setters) ----
impl ExfatInode {
    pub(super) fn prepare_directory_metadata_refresh_with_guards(
        &self,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
        timestamp: Duration,
    ) -> Result<Option<PreparedEntrySetWrite>> {
        self.prepare_rewritten_entry_set_write_with_guard(
            self_inode_state_guard,
            parent_inode_state_guard,
            boot_region,
            |entry_view| {
                let (timestamp_bytes, ten_ms_increment, encoded_utc_offset_byte) =
                    Self::encoded_exfat_timestamp_fields(
                        timestamp,
                        entry_view.last_modified_timestamp().utc_offset_byte(),
                    )?;
                let mut mutable_entry_set = entry_view.to_mutable();
                mutable_entry_set.set_last_modified_timestamp(FileEntryTimestamp::new(
                    timestamp_bytes,
                    Some(ten_ms_increment),
                    encoded_utc_offset_byte,
                ));
                Ok(Some(mutable_entry_set.into_bytes()))
            },
        )
    }

    pub(super) fn refresh_cached_metadata_from_entry_view(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
        entry_view: FileEntrySetView<'_>,
        boot_region: &BootRegion,
    ) -> Result<()> {
        let (inode_type, _first_cluster, data_length, _no_fat_chain) =
            entry_view.child_metadata(boot_region)?;
        let create_at = Self::decoded_exfat_timestamp(
            entry_view.create_timestamp().timestamp_bytes(),
            entry_view.create_timestamp().ten_ms_increment(),
            entry_view.create_timestamp().utc_offset_byte(),
        )?;
        let last_access_at = Self::decoded_exfat_timestamp(
            entry_view.last_accessed_timestamp().timestamp_bytes(),
            entry_view.last_accessed_timestamp().ten_ms_increment(),
            entry_view.last_accessed_timestamp().utc_offset_byte(),
        )?;
        let last_modify_at = Self::decoded_exfat_timestamp(
            entry_view.last_modified_timestamp().timestamp_bytes(),
            entry_view.last_modified_timestamp().ten_ms_increment(),
            entry_view.last_modified_timestamp().utc_offset_byte(),
        )?;
        let allocated_sectors = Self::regular_file_allocated_sectors(boot_region, data_length)?;
        inode_state_guard.with_metadata_mut(|metadata| {
            if metadata.type_ != inode_type {
                return Err(invalid_on_disk_layout());
            }
            let writable_bits = metadata.mode & mkmod!(a+w);
            metadata.mode = chmod!(metadata.mode, a-w);
            if !entry_view.is_read_only() {
                metadata.mode |= writable_bits;
            }
            metadata.birth_at = Some(create_at);
            metadata.last_access_at = last_access_at;
            metadata.last_meta_change_at = last_modify_at;
            metadata.last_modify_at = last_modify_at;
            metadata.nr_sectors_allocated = allocated_sectors;
            metadata.size = data_length;
            Ok(())
        })?;
        Ok(())
    }

    // Write path

    pub(super) fn set_mode_impl(&self, mode: InodeMode) -> Result<()> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let mut fs_state = fs.fs_state.write();
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return_errno!(Errno::EIO);
        }
        if mount_state.options.fs_flags.contains(FsFlags::RDONLY) {
            return_errno!(Errno::EROFS);
        }

        let (discovered_type, parent) = {
            let inode_state_guard = self.inode_state_read_guard();
            (
                inode_state_guard.metadata().type_,
                inode_state_guard.parent(),
            )
        };
        let mut guarded_inodes = vec![self];
        if matches!(discovered_type, InodeType::Dir | InodeType::File)
            && let Some(parent) = parent.as_ref()
        {
            guarded_inodes.push(parent.as_ref());
        }
        let inode_guards = Self::inode_write_guards_in_lock_order(guarded_inodes);
        let self_inode_state_guard = inode_guards
            .iter()
            .find(|guard| guard.guards_inode(self))
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let inode_type = self_inode_state_guard.metadata().type_;
        if !matches!(inode_type, InodeType::Dir | InodeType::File) {
            self_inode_state_guard.with_metadata_mut(|metadata| metadata.mode = mode);
            return Ok(());
        }

        let requested_writable = mode.intersects(mkmod!(a+w));
        let current_writable = self_inode_state_guard
            .metadata()
            .mode
            .intersects(mkmod!(a+w));
        if inode_type == InodeType::Dir
            && self_inode_state_guard
                .dir_entry_stream()
                .data_length
                .is_none()
        {
            if requested_writable == current_writable {
                return Ok(());
            }
            return_errno!(Errno::EOPNOTSUPP);
        }
        if requested_writable == current_writable {
            return Ok(());
        }
        let parent = parent.as_ref().ok_or_else(|| Error::new(Errno::EIO))?;
        if !self_inode_state_guard
            .parent()
            .is_some_and(|admitted_parent| Arc::ptr_eq(&admitted_parent, parent))
        {
            return_errno!(Errno::EIO);
        }
        let parent_inode_state_guard = inode_guards
            .iter()
            .find(|guard| guard.guards_inode(parent.as_ref()))
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let boot_region = fs.immutable_boot_region();
        let _allocation_guard = fs.allocation_read_guard()?;
        let update_result = (|| {
            fs.publish_dirty_admission(&mut fs_state)?;

            self.rewrite_inode_entry_set_with_guards(
                &mut fs_state,
                self_inode_state_guard,
                parent_inode_state_guard,
                &boot_region,
                |entry_view| {
                    if requested_writable != entry_view.is_read_only() {
                        return Ok(None);
                    }
                    let mut file_attributes = entry_view.file_attributes();
                    if inode_type == InodeType::Dir {
                        file_attributes |= direntry::FILE_ATTRIBUTE_DIRECTORY;
                    }
                    if requested_writable {
                        file_attributes &= !direntry::FILE_ATTRIBUTE_READ_ONLY;
                    } else {
                        file_attributes |= direntry::FILE_ATTRIBUTE_READ_ONLY;
                    }
                    let mut mutable_entry_set = entry_view.to_mutable();
                    mutable_entry_set.set_file_attributes(file_attributes);
                    Ok(Some(mutable_entry_set.into_bytes()))
                },
                |metadata| {
                    if inode_type == InodeType::Dir {
                        let writable_bits = metadata.mode & mkmod!(a+w);
                        metadata.mode = chmod!(metadata.mode, a-w);
                        if requested_writable {
                            metadata.mode |= writable_bits;
                        }
                    } else {
                        metadata.mode = chmod!(metadata.mode, a-w);
                        if requested_writable {
                            metadata.mode |= mkmod!(u+w);
                        }
                    }
                },
            )
        })();
        if update_result.is_err() {
            ExfatFs::mark_mount_dirty_after_failure(&mut fs_state);
        }
        let durable_updated = update_result?;
        if durable_updated {
            self_inode_state_guard.with_metadata_mut(|metadata| {
                metadata.last_meta_change_at = RealTimeCoarseClock::get().read_time();
            });
            self.mark_metadata_dirty(self_inode_state_guard);
        }
        Ok(())
    }

    pub(super) fn set_atime_impl(&self, time: Duration) {
        self.rewrite_timestamp(InodeTimestampField::Accessed, time);
    }

    pub(super) fn update_atime_after_eligible_read(&self) {
        let Some(fs) = self.fs.upgrade() else {
            return;
        };
        let mut fs_state = fs.fs_state.write();
        let Some(mount_state) = fs_state.mount_state.as_ref() else {
            return;
        };
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
            || mount_state.options.fs_flags.contains(FsFlags::RDONLY)
        {
            return;
        }
        let operation_time = RealTimeClock::get().read_time();
        let (discovered_type, is_root_directory, parent) = {
            let inode_state_guard = self.inode_state_read_guard();
            (
                inode_state_guard.metadata().type_,
                inode_state_guard.dir_entry_stream().data_length.is_none(),
                inode_state_guard.parent(),
            )
        };
        if !matches!(discovered_type, InodeType::Dir | InodeType::File) {
            return;
        }
        if discovered_type == InodeType::Dir && is_root_directory {
            let inode_state_guard = self.inode_state_write_guard();
            if inode_state_guard.metadata().type_ == InodeType::Dir
                && inode_state_guard.dir_entry_stream().data_length.is_none()
            {
                inode_state_guard
                    .with_metadata_mut(|metadata| metadata.last_access_at = operation_time);
            }
            return;
        }

        let mut guarded_inodes = vec![self];
        if let Some(parent) = parent.as_ref() {
            guarded_inodes.push(parent.as_ref());
        }
        let inode_guards = Self::inode_write_guards_in_lock_order(guarded_inodes);
        let Some(self_inode_state_guard) =
            inode_guards.iter().find(|guard| guard.guards_inode(self))
        else {
            return;
        };
        if !matches!(
            self_inode_state_guard.metadata().type_,
            InodeType::Dir | InodeType::File
        ) || (self_inode_state_guard.metadata().type_ == InodeType::Dir
            && self_inode_state_guard
                .dir_entry_stream()
                .data_length
                .is_none())
        {
            return;
        }
        self_inode_state_guard
            .with_metadata_mut(|metadata| metadata.last_access_at = operation_time);

        let Some(parent) = parent.as_ref() else {
            return;
        };
        if !self_inode_state_guard
            .parent()
            .is_some_and(|admitted_parent| Arc::ptr_eq(&admitted_parent, parent))
        {
            return;
        }
        let Some(parent_inode_state_guard) = inode_guards
            .iter()
            .find(|guard| guard.guards_inode(parent.as_ref()))
        else {
            return;
        };
        let Ok(_allocation_guard) = fs.allocation_read_guard() else {
            return;
        };
        let boot_region = fs.immutable_boot_region();
        let rewrite_result = (|| {
            fs.publish_dirty_admission(&mut fs_state)?;
            self.rewrite_inode_entry_set_with_guards(
                &mut fs_state,
                self_inode_state_guard,
                parent_inode_state_guard,
                &boot_region,
                |entry_view| {
                    let (timestamp_bytes, _ten_ms_increment, encoded_utc_offset_byte) =
                        Self::encoded_exfat_timestamp_fields(
                            operation_time,
                            entry_view.last_accessed_timestamp().utc_offset_byte(),
                        )?;
                    let mut mutable_entry_set = entry_view.to_mutable();
                    mutable_entry_set.set_last_accessed_timestamp(FileEntryTimestamp::new(
                        timestamp_bytes,
                        None,
                        encoded_utc_offset_byte,
                    ));
                    Ok(Some(mutable_entry_set.into_bytes()))
                },
                |metadata| metadata.last_access_at = operation_time,
            )
        })();
        if rewrite_result.is_err() {
            ExfatFs::mark_mount_dirty_after_failure(&mut fs_state);
        }
        if rewrite_result.is_ok_and(|updated| updated) {
            self.mark_metadata_dirty(self_inode_state_guard);
        }
    }

    pub(super) fn set_mtime_impl(&self, time: Duration) {
        self.rewrite_timestamp(InodeTimestampField::Modified, time);
    }

    pub(super) fn set_ctime_impl(&self, time: Duration) {
        let Some(fs) = self.fs.upgrade() else {
            return;
        };
        let fs_state = fs.fs_state.read();
        let Some(mount_state) = fs_state.mount_state.as_ref() else {
            return;
        };
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return;
        }
        if mount_state.options.fs_flags.contains(FsFlags::RDONLY) {
            return;
        }

        let inode_state_guard = self.inode_state_write_guard();
        if inode_state_guard.metadata().type_ != InodeType::Dir {
            inode_state_guard.with_metadata_mut(|metadata| metadata.last_meta_change_at = time);
        }
    }

    pub(super) fn set_owner_impl(&self, uid: Uid) -> Result<()> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let fs_state = fs.fs_state.read();
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return_errno!(Errno::EIO);
        }
        if mount_state.options.fs_flags.contains(FsFlags::RDONLY) {
            return_errno!(Errno::EROFS);
        }
        let inode_state_guard = self.inode_state_write_guard();
        if !matches!(
            inode_state_guard.metadata().type_,
            InodeType::Dir | InodeType::File
        ) {
            inode_state_guard.with_metadata_mut(|metadata| metadata.uid = uid);
            return Ok(());
        }
        if inode_state_guard.metadata().uid == uid {
            return Ok(());
        }
        return_errno!(Errno::EPERM);
    }

    pub(super) fn set_group_impl(&self, gid: Gid) -> Result<()> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let fs_state = fs.fs_state.read();
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return_errno!(Errno::EIO);
        }
        if mount_state.options.fs_flags.contains(FsFlags::RDONLY) {
            return_errno!(Errno::EROFS);
        }

        let inode_state_guard = self.inode_state_write_guard();
        if !matches!(
            inode_state_guard.metadata().type_,
            InodeType::Dir | InodeType::File
        ) {
            inode_state_guard.with_metadata_mut(|metadata| metadata.gid = gid);
            return Ok(());
        }
        if inode_state_guard.metadata().gid == gid {
            return Ok(());
        }
        return_errno!(Errno::EPERM);
    }
}

// ---- entry_rewrite (timestamp + directory metadata refresh) ----
impl ExfatInode {
    fn rewrite_timestamp(&self, field_kind: InodeTimestampField, time: Duration) {
        let Some(fs) = self.fs.upgrade() else {
            return;
        };
        // TODO: These timestamp setters still admit through the generic mounted-mutation gate and
        // reuse the currently stored exFAT UTC-offset byte because `MountOptions` does not
        // yet own explicit `allow_utime` / timezone policy. Once `MountOptions` exposes that
        // policy under `ExfatFs`, route timestamp admission and UTC-offset selection through it.
        let mut fs_state = fs.fs_state.write();
        let Some(mount_state) = fs_state.mount_state.as_ref() else {
            return;
        };
        let boot_region = fs.immutable_boot_region();
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return;
        }
        if mount_state.options.fs_flags.contains(FsFlags::RDONLY) {
            return;
        }

        let (discovered_type, is_root_directory, parent) = {
            let inode_state_guard = self.inode_state_read_guard();
            (
                inode_state_guard.metadata().type_,
                inode_state_guard.dir_entry_stream().data_length.is_none(),
                inode_state_guard.parent(),
            )
        };
        if !matches!(discovered_type, InodeType::Dir | InodeType::File) {
            let inode_state_guard = self.inode_state_write_guard();
            match field_kind {
                InodeTimestampField::Accessed => {
                    inode_state_guard.with_metadata_mut(|metadata| metadata.last_access_at = time)
                }
                InodeTimestampField::Modified => {
                    inode_state_guard.with_metadata_mut(|metadata| metadata.last_modify_at = time)
                }
            }
            return;
        }
        if discovered_type == InodeType::Dir && is_root_directory {
            let inode_state_guard = self.inode_state_write_guard();
            if inode_state_guard.metadata().type_ == InodeType::Dir
                && inode_state_guard.dir_entry_stream().data_length.is_none()
            {
                return;
            }
        }
        let Some(parent) = parent else {
            return;
        };
        let inode_guards = Self::inode_write_guards_in_lock_order(vec![self, parent.as_ref()]);
        let Some(self_inode_state_guard) =
            inode_guards.iter().find(|guard| guard.guards_inode(self))
        else {
            return;
        };
        if !matches!(
            self_inode_state_guard.metadata().type_,
            InodeType::Dir | InodeType::File
        ) || !self_inode_state_guard
            .parent()
            .is_some_and(|admitted_parent| Arc::ptr_eq(&admitted_parent, &parent))
        {
            return;
        }
        let Some(parent_inode_state_guard) = inode_guards
            .iter()
            .find(|guard| guard.guards_inode(parent.as_ref()))
        else {
            return;
        };
        let Ok(_allocation_guard) = fs.allocation_read_guard() else {
            return;
        };

        let normalized_modify_time = Cell::new(None);
        let rewrite_result = (|| {
            fs.publish_dirty_admission(&mut fs_state)?;

            self.rewrite_inode_entry_set_with_guards(
                &mut fs_state,
                self_inode_state_guard,
                parent_inode_state_guard,
                &boot_region,
                |entry_view| {
                    let mut mutable_entry_set = entry_view.to_mutable();
                    match field_kind {
                        InodeTimestampField::Accessed => {
                            let (timestamp_bytes, _ten_ms_increment, encoded_utc_offset_byte) =
                                Self::encoded_exfat_timestamp_fields(
                                    time,
                                    entry_view.last_accessed_timestamp().utc_offset_byte(),
                                )?;
                            mutable_entry_set.set_last_accessed_timestamp(FileEntryTimestamp::new(
                                timestamp_bytes,
                                None,
                                encoded_utc_offset_byte,
                            ));
                        }
                        InodeTimestampField::Modified => {
                            let (timestamp_bytes, ten_ms_increment, encoded_utc_offset_byte) =
                                Self::encoded_exfat_timestamp_fields(
                                    time,
                                    entry_view.last_modified_timestamp().utc_offset_byte(),
                                )?;
                            let normalized_timestamp = Self::decoded_exfat_timestamp(
                                timestamp_bytes,
                                Some(ten_ms_increment),
                                encoded_utc_offset_byte,
                            )?;
                            normalized_modify_time.set(Some(normalized_timestamp));
                            mutable_entry_set.set_last_modified_timestamp(FileEntryTimestamp::new(
                                timestamp_bytes,
                                Some(ten_ms_increment),
                                encoded_utc_offset_byte,
                            ));
                        }
                    }
                    Ok(Some(mutable_entry_set.into_bytes()))
                },
                |metadata| match field_kind {
                    InodeTimestampField::Accessed => {
                        // Keep live atime precise; exFAT's durable access timestamp is two-second
                        // granular and has no 10ms increment field.
                        metadata.last_access_at = time;
                    }
                    InodeTimestampField::Modified => {
                        metadata.last_meta_change_at = time;
                        metadata.last_modify_at = normalized_modify_time.get().unwrap_or(time);
                    }
                },
            )
        })();
        if rewrite_result.is_err() {
            ExfatFs::mark_mount_dirty_after_failure(&mut fs_state);
        }
        if rewrite_result.is_ok_and(|updated| updated) {
            self.mark_metadata_dirty(self_inode_state_guard);
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Directory metadata refresh must keep caller-owned guard proof, boot-region timestamp context, the optional prepared write, and persistence-recovery classification explicit for rollback handling."
    )]
    pub(super) fn refresh_directory_metadata_after_namespace_mutation_with_guards(
        &self,
        fs_state: &mut FsState,
        boot_region: &BootRegion,
        timestamp: Duration,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: Option<&InodeStateWriteGuard<'_>>,
        prepared_entry_set_write: Option<PreparedEntrySetWrite>,
        recovery: PersistenceRecovery,
    ) -> Result<()> {
        if self_inode_state_guard.metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        if self_inode_state_guard
            .dir_entry_stream()
            .data_length
            .is_none()
        {
            self_inode_state_guard.with_metadata_mut(|metadata| {
                metadata.last_meta_change_at = timestamp;
                metadata.last_modify_at = timestamp;
            });
            self.mark_metadata_dirty(self_inode_state_guard);
            return Ok(());
        }

        let parent_inode_state_guard = parent_inode_state_guard.ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "ordinary exFAT directory refresh requires parent write-guard proof",
            )
        })?;
        let classified_update = if let Some(prepared_entry_set_write) = prepared_entry_set_write {
            let parent_inode = self_inode_state_guard.parent().ok_or_else(|| {
                Error::with_message(Errno::EIO, "ordinary exFAT inode parent is not mounted")
            })?;
            if !parent_inode_state_guard.guards_inode(parent_inode.as_ref()) {
                return Err(Error::new(Errno::EINVAL));
            }
            self.persist_prepared_entry_set_write_classified(
                fs_state,
                prepared_entry_set_write,
                parent_inode.as_ref(),
                parent_inode_state_guard.metadata(),
                recovery,
            )
        } else {
            self.rewrite_validated_entry_set_with_guard_classified(
                fs_state,
                self_inode_state_guard,
                parent_inode_state_guard,
                boot_region,
                |entry_view| {
                    let (timestamp_bytes, ten_ms_increment, encoded_utc_offset_byte) =
                        Self::encoded_exfat_timestamp_fields(
                            timestamp,
                            entry_view.last_modified_timestamp().utc_offset_byte(),
                        )?;
                    let mut mutable_entry_set = entry_view.to_mutable();
                    mutable_entry_set.set_last_modified_timestamp(FileEntryTimestamp::new(
                        timestamp_bytes,
                        Some(ten_ms_increment),
                        encoded_utc_offset_byte,
                    ));
                    Ok(Some(mutable_entry_set.into_bytes()))
                },
                recovery,
            )
        };
        match classified_update {
            Ok(Ok(durable_updated)) => {
                if durable_updated {
                    self_inode_state_guard.with_metadata_mut(|metadata| {
                        metadata.last_meta_change_at = timestamp;
                        metadata.last_modify_at = timestamp;
                    });
                    self.mark_metadata_dirty(self_inode_state_guard);
                }
                Ok(())
            }
            Ok(Err(error)) => {
                self_inode_state_guard.with_metadata_mut(|metadata| {
                    metadata.last_meta_change_at = timestamp;
                    metadata.last_modify_at = timestamp;
                });
                self.mark_metadata_dirty(self_inode_state_guard);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn rewrite_inode_entry_set_with_guards(
        &self,
        fs_state: &mut FsState,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
        rewrite_entry_set_fn: impl FnOnce(FileEntrySetView<'_>) -> Result<Option<Vec<u8>>>,
        update_metadata_fn: impl FnOnce(&mut Metadata),
    ) -> Result<bool> {
        let classified_update = self.rewrite_validated_entry_set_with_guard_classified(
            fs_state,
            self_inode_state_guard,
            parent_inode_state_guard,
            boot_region,
            rewrite_entry_set_fn,
            PersistenceRecovery::RollbackAllowed,
        );
        match classified_update {
            Ok(Ok(durable_updated)) => {
                if durable_updated {
                    self_inode_state_guard.with_metadata_mut(update_metadata_fn);
                }
                Ok(durable_updated)
            }
            Ok(Err(error)) => {
                self_inode_state_guard.with_metadata_mut(update_metadata_fn);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn publish_live_regular_file_entry_set(
        &self,
        fs_state: &mut FsState,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
    ) -> Result<bool> {
        if self_inode_state_guard.metadata().type_ != InodeType::File {
            return_errno!(Errno::EOPNOTSUPP);
        }

        let cluster_map = self_inode_state_guard.dir_entry_stream();
        let last_modify_at = self_inode_state_guard.metadata().last_modify_at;
        let durable_updated = self.rewrite_inode_entry_set_with_guards(
            fs_state,
            self_inode_state_guard,
            parent_inode_state_guard,
            boot_region,
            |entry_view| {
                let (inode_type, _first_cluster, _data_length, _no_fat_chain) =
                    entry_view.child_metadata(boot_region)?;
                if inode_type != InodeType::File || entry_view.is_directory() {
                    return Err(invalid_on_disk_layout());
                }

                let (timestamp_bytes, hundredths_increment, encoded_utc_offset_byte) =
                    Self::encoded_exfat_timestamp_fields(
                        last_modify_at,
                        entry_view.last_modified_timestamp().utc_offset_byte(),
                    )?;
                let mut mutable_entry_set = entry_view.to_mutable();
                mutable_entry_set.set_cluster_map(&cluster_map)?;
                mutable_entry_set.set_last_modified_timestamp(FileEntryTimestamp::new(
                    timestamp_bytes,
                    Some(hundredths_increment),
                    encoded_utc_offset_byte,
                ));
                Ok(Some(mutable_entry_set.into_bytes()))
            },
            |_| {},
        )?;
        Ok(durable_updated)
    }
}
