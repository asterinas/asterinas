// SPDX-License-Identifier: MPL-2.0

//! Maps regular-file clusters into page-cache I/O ranges and serves cached reads.
//!
//! This module owns the regular-file read path after inode state has been admitted.
//! It clips read ranges against validated file length,
//! translates stream clusters into page-cache-visible I/O ranges,
//! and routes reads through the shared page-cache/backend contract.
//!
//! Its entry points cover read dispatch,
//! regular-file mapping validation,
//! and cluster lookup for the current read window.
//! The core data model is the validated regular-file cluster map
//! plus the page-sized I/O ranges derived from that map.
//!
//! Locking matters because read paths may reuse cached mapping state
//! but must not publish new page-cache context through a read-only admission path.
//! This module does not own persistence,
//! and short reads are bounded by the validated stream length and available mapping.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 5.1, 7.6.5, 7.6.6, and 7.6.7,
//! plus `crate::vm::page_cache::PageCache`.

use ostd::mm::VmIo;

use super::ExfatInode;
use crate::{
    fs::file::{InodeType, StatusFlags},
    prelude::*,
};

impl ExfatInode {
    pub(super) fn read_at_impl(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let read_result = (|| {
            let fs = self.fs.upgrade().ok_or_else(|| {
                Error::with_message(Errno::EIO, "exFAT filesystem is not mounted")
            })?;
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
            let inode_state_guard = self.inode_state_read_guard();
            match inode_state_guard.metadata().type_ {
                InodeType::Dir => return_errno!(Errno::EISDIR),
                InodeType::File => {}
                _ => return_errno!(Errno::EOPNOTSUPP),
            }
            let allocation_guard = fs.allocation_read_guard()?;

            let (_cluster_map, data_length, _valid_data_length) =
                self.cluster_map_for_admitted_read(&inode_state_guard, &allocation_guard)?;
            if !writer.has_avail() || data_length == 0 {
                return Ok(0);
            }

            let page_cache = self
                .page_cache_handle(inode_state_guard.metadata())
                .ok_or_else(|| {
                    Error::with_message(Errno::EIO, "regular exFAT file has no page cache")
                })?;
            let read_start = offset.min(data_length);
            let read_end = offset
                .checked_add(writer.avail())
                .ok_or_else(|| Error::new(Errno::EINVAL))?
                .min(data_length);
            if read_start == read_end {
                return Ok(0);
            }
            let read_len = read_end - read_start;

            let (read_result, copied_len) = {
                let mut limited_writer = writer.clone_exclusive();
                limited_writer.limit(read_len);
                let read_result = page_cache
                    .read(read_start, &mut limited_writer)
                    .map_err(Error::from);
                let copied_len = read_len - limited_writer.avail();
                (read_result, copied_len)
            };
            if let Err(error) = read_result
                && copied_len == 0
            {
                return Err(error);
            }
            writer.skip(copied_len);
            Ok(copied_len)
        })();
        if read_result.is_ok() {
            self.update_atime_after_eligible_read();
        }
        read_result
    }
}
