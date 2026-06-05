// SPDX-License-Identifier: MPL-2.0

//! Methods and constructors for `VirtioFsInode`.

use core::time::Duration;

use aster_fuse::{
    EntryReply, FuseAttrReply, FuseDirEntry, FuseFileHandle, FuseOpenFlags, GetattrFlags, ReadReq,
    ReleaseFlags, ReleaseKind, SetattrReq, SetattrValid, WriteFlags, WriteReq,
    ops::{
        getattr::{GetattrOperation, GetattrReq},
        lookup::LookupOperation,
        open::{OpenOperation, OpenReq, OpendirOperation},
        release::ReleaseOptions,
        setattr::SetattrOperation,
    },
};
use aster_virtio::device::filesystem::device::AttrVersion;
use ostd::mm::{VmIo, io::util::HasVmReaderWriter};

use super::{
    super::{
        dir::VirtioFsDir,
        file::{CachePolicy, VirtioFsFile},
        open_handle::VirtioFsOpenHandle,
        valid_until,
    },
    TimeField, VirtioFsInode, WriteOffset,
    metadata::StaleAttrAction,
};
use crate::{
    fs::{
        file::{AccessMode, PerOpenFileOps, StatusFlags},
        utils::DirentVisitor,
    },
    prelude::*,
    thread::work_queue::{self, WorkPriority},
    time::clocks::MonotonicCoarseClock,
};

/// Use one page for each `FUSE_READDIR` request.
const FUSE_READDIR_BUF_SIZE: u32 = 4096;

impl VirtioFsInode {
    /// Reads file data through the page cache.
    pub(in crate::fs::fs_impls::virtiofs) fn cached_read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        fh: FuseFileHandle,
        flags: u32,
    ) -> Result<usize> {
        if self.inner.read().page_cache().is_none() {
            return self.direct_read_at(offset, writer, fh, flags);
        }

        // FIXME: The virtio-fs session currently always requests `AUTO_INVAL_DATA`,
        // so cached reads refresh attributes before checking the cached file size.
        // If this flag becomes optional, reads that stay below EOF should be allowed
        // to use still-valid cached attributes.
        self.revalidate_attr(fh)?;

        let inner = self.inner.read();
        let file_size = self.size();
        let start = file_size.min(offset);
        let end = file_size.min(offset.saturating_add(writer.avail()));
        let read_len = end - start;
        if read_len == 0 {
            return Ok(0);
        }

        let mut limited_writer = writer.clone_exclusive();
        limited_writer.limit(read_len);
        let page_cache = inner.page_cache().unwrap();
        page_cache.read(start, &mut limited_writer)?;
        writer.skip(read_len);

        Ok(read_len)
    }

    /// Reads file data directly from the server.
    pub(in crate::fs::fs_impls::virtiofs) fn direct_read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        fh: FuseFileHandle,
        flags: u32,
    ) -> Result<usize> {
        // FIXME: Direct reads do not support short reads yet, so `FUSE_READ`
        // must request the actual bytes to read instead of the caller's maximum
        // buffer size. Refresh attributes before using the cached file size.
        self.revalidate_attr(fh)?;

        let _inner = self.inner.read();
        let file_size = self.size();
        let start = file_size.min(offset);
        let end = file_size.min(offset.saturating_add(writer.avail()));
        let read_len = end - start;

        if read_len == 0 {
            return Ok(0);
        }

        // FIXME: A direct read must observe writes already staged in the page cache.
        // With the current write-through policy, this usually has no dirty pages to
        // submit, but it is still required for future write-back semantics.

        let fs = self.fs_ref();
        let data_buf = fs.session().alloc_read_buf(read_len)?;

        let copied = fs.session().read(
            self.nodeid(),
            ReadReq::new(fh, start as u64, read_len as u32, flags),
            data_buf.clone(),
        )?;

        let mut segment_reader = data_buf.reader()?;
        segment_reader.limit(copied);
        segment_reader
            .read_fallible(writer)
            .map_err(|(err, _)| Error::from(err))?;

        Ok(copied)
    }

    /// Writes file data through the page cache.
    ///
    /// Cached writes use write-through semantics: they copy user bytes into the
    /// page cache, flush the dirtied range to the server, and commit metadata
    /// only after writeback succeeds.
    pub(in crate::fs::fs_impls::virtiofs) fn cached_write_at(
        &self,
        write_offset: WriteOffset,
        reader: &mut VmReader,
        fh: FuseFileHandle,
        flags: u32,
    ) -> Result<usize> {
        if self.inner.read().page_cache().is_none() {
            return self.direct_write_at(write_offset, reader, fh, flags);
        }
        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(0);
        }

        let mut inner = self.inner.write();
        let offset = self.resolve_write_offset(write_offset);
        let requested_end = offset
            .checked_add(write_len)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs write size overflow"))?;

        let old_size = self.size();
        let page_cache = inner.page_cache().unwrap();

        if requested_end > old_size {
            // Extend the visible EOF before growing the page cache.
            self.set_size(requested_end);
            page_cache
                .resize(requested_end, old_size)
                .expect("expanding the page cache should not fail");
        }

        let mut write_through_page_cache = || -> Result<()> {
            page_cache.write(offset, reader).map_err(Error::from)?;
            page_cache.flush_range(offset..requested_end)
        };

        if let Err(err) = write_through_page_cache() {
            if requested_end > old_size {
                // Roll back in truncate order: shrink the page cache before
                // restoring the old visible EOF.
                page_cache.resize(old_size, requested_end)?;
                self.set_size(old_size);
            }
            return Err(err);
        }

        let new_size = self.size().max(requested_end);
        let attr_version = self.fs_ref().session().bump_attr_version();

        inner.commit_local_write(new_size, attr_version);

        Ok(write_len)
    }

    /// Writes file data directly to the server.
    pub(in crate::fs::fs_impls::virtiofs) fn direct_write_at(
        &self,
        write_offset: WriteOffset,
        reader: &mut VmReader,
        fh: FuseFileHandle,
        flags: u32,
    ) -> Result<usize> {
        let write_len = reader.remain();

        let mut inner = self.inner.write();
        let offset = self.resolve_write_offset(write_offset);
        let write_end = offset
            .checked_add(write_len)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs write size overflow"))?;

        if let Some(page_cache) = inner.page_cache() {
            page_cache.invalidate_range(offset..write_end)?;
        };

        let written = self.do_direct_write(offset, reader, fh, flags)?;

        // TODO: Do `evict_range` again after write to prevent stale data from
        // being loaded into the page cache during asynchronous read-ahead.

        let new_size = offset
            .checked_add(written)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "virtiofs write size overflow"))?;

        let new_size = self.size().max(new_size);
        let attr_version = self.fs_ref().session().bump_attr_version();
        inner.commit_local_write(new_size, attr_version);
        self.set_size(new_size);

        Ok(written)
    }

    fn resolve_write_offset(&self, write_offset: WriteOffset) -> usize {
        match write_offset {
            WriteOffset::Absolute(offset) => offset,
            WriteOffset::Append => self.size(),
        }
    }

    pub(super) fn open_transient_handle(
        &self,
        access_mode: AccessMode,
    ) -> Result<Arc<VirtioFsOpenHandle>> {
        let fs = self.fs_ref();
        let flags = access_mode as u32;
        let open_out = fs
            .session()
            .do_fuse_op(self.nodeid(), OpenOperation::new(OpenReq::new(flags)))?;
        Ok(VirtioFsOpenHandle::new(
            open_out.fh(),
            self.nodeid(),
            access_mode,
            StatusFlags::empty(),
            open_out.open_flags(),
            self.fs.clone(),
            ReleaseOptions::new(ReleaseKind::File, ReleaseFlags::empty()),
        ))
    }

    fn do_direct_write(
        &self,
        offset: usize,
        reader: &mut VmReader,
        fh: FuseFileHandle,
        flags: u32,
    ) -> Result<usize> {
        let fs = self.fs_ref();
        let max_write = fs.session().max_write() as usize;
        let mut total_written = 0usize;

        // `FUSE_WRITE` replies carry the accepted byte count. Submit and wait
        // for one chunk at a time. Once any bytes are accepted, a later error
        // is reported as a successful partial write.
        while reader.has_remain() {
            let write_size = reader.remain().min(max_write);
            let data_buf = match fs.session().alloc_write_buf(write_size) {
                Ok(data_buf) => data_buf,
                Err(_) if total_written > 0 => return Ok(total_written),
                Err(err) => return Err(err.into()),
            };

            let mut segment_writer = data_buf.writer().unwrap();
            let mut request_reader = reader.clone();
            request_reader.limit(write_size);
            if let Err((err, _)) = segment_writer.write_fallible(&mut request_reader) {
                if total_written > 0 {
                    return Ok(total_written);
                }

                return Err(err.into());
            }

            let request_offset = match offset.checked_add(total_written) {
                Some(request_offset) => request_offset,
                None if total_written > 0 => return Ok(total_written),
                None => {
                    return Err(Error::with_message(
                        Errno::EOVERFLOW,
                        "virtiofs write offset overflow",
                    ));
                }
            };
            let ret = fs
                .session()
                .write(
                    self.nodeid(),
                    WriteReq::new(
                        fh,
                        request_offset as u64,
                        write_size as u32,
                        flags,
                        WriteFlags::empty(),
                    ),
                    data_buf,
                )
                .map_err(Error::from);

            let written = match ret {
                Ok(written) => written,
                Err(_) if total_written > 0 => return Ok(total_written),
                Err(err) => return Err(err),
            };

            if written > write_size {
                if total_written > 0 {
                    return Ok(total_written);
                }

                return_errno_with_message!(Errno::EIO, "virtiofs write response is too large");
            }
            if written == 0 {
                break;
            }

            let Some(new_total_written) = total_written.checked_add(written) else {
                return Ok(total_written);
            };
            reader.skip(written);
            total_written = new_total_written;
            if written < write_size {
                break;
            }
        }

        Ok(total_written)
    }

    pub(super) fn open_file(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        let inode = self
            .weak_self
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "virtiofs inode is unavailable"))?;
        let fs = self.fs_ref();

        let open_out = fs.session().do_fuse_op(
            self.nodeid(),
            OpenOperation::new(OpenReq::new((access_mode as u32) | status_flags.bits())),
        )?;
        let cache_policy = if self.inner.read().page_cache().is_some()
            && !open_out
                .open_flags()
                .contains(FuseOpenFlags::FOPEN_DIRECT_IO)
        {
            CachePolicy::Cached
        } else {
            CachePolicy::Direct
        };
        let open_handle = VirtioFsOpenHandle::new(
            open_out.fh(),
            self.nodeid(),
            access_mode,
            status_flags,
            open_out.open_flags(),
            self.fs.clone(),
            ReleaseOptions::new(ReleaseKind::File, ReleaseFlags::RELEASE_FLUSH),
        );
        if !open_out
            .open_flags()
            .contains(FuseOpenFlags::FOPEN_KEEP_CACHE)
            && let Err(err) = self.invalidate_whole_page_cache()
        {
            return Err(err);
        }

        // Cache the open handle for later page cache I/O.
        if cache_policy == CachePolicy::Cached {
            self.open_handles.insert(&open_handle);
        }

        Ok(Box::new(VirtioFsFile::new(
            inode,
            open_handle,
            cache_policy,
        )))
    }

    pub(super) fn open_directory(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        let inode = self
            .weak_self
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "virtiofs inode is unavailable"))?;
        let fs = self.fs_ref();

        let open_out = fs
            .session()
            .do_fuse_op(self.nodeid(), OpendirOperation::new(OpenReq::new(0)))?;
        let open_handle = VirtioFsOpenHandle::new(
            open_out.fh(),
            self.nodeid(),
            access_mode,
            status_flags,
            open_out.open_flags(),
            self.fs.clone(),
            ReleaseOptions::new(ReleaseKind::Directory, ReleaseFlags::empty()),
        );

        Ok(Box::new(VirtioFsDir::new(inode, open_handle)))
    }

    /// Commits an `EntryReply` reply for this cached inode.
    ///
    /// `EntryReply` replies carry both attributes and one new lookup reference.
    /// Updating the two together keeps the client-side `nlookup` mirror in sync
    /// with the server-side count.
    pub(super) fn commit_entry_reply(
        &self,
        entry_reply: &EntryReply,
        request_attr_version: AttrVersion,
        stale_action: StaleAttrAction,
    ) -> Result<()> {
        debug_assert_eq!(entry_reply.nodeid(), self.nodeid());
        self.lookup_count.acquire();

        self.commit_attr_reply(
            FuseAttrReply::from(entry_reply),
            request_attr_version,
            stale_action,
        )
    }

    /// Reads directory entries and expires this directory's attribute cache.
    ///
    /// `READDIR` does not return a complete attribute reply for the directory,
    /// but successful directory reads may observe server-side changes. Expiring
    /// attributes forces later metadata-sensitive operations to revalidate.
    pub(in crate::fs::fs_impls::virtiofs) fn readdir(
        &self,
        fh: FuseFileHandle,
        offset: usize,
        flags: u32,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let fs = self.fs_ref();
        let data_buf = fs
            .session()
            .alloc_read_buf(FUSE_READDIR_BUF_SIZE as usize)?;
        let entries: Vec<FuseDirEntry> = fs.session().readdir(
            self.nodeid(),
            ReadReq::new(fh, offset as u64, FUSE_READDIR_BUF_SIZE, flags),
            data_buf,
        )?;

        let offset_read = {
            let try_readdir_fn = |offset: &mut usize,
                                  visitor: &mut dyn DirentVisitor|
             -> Result<()> {
                for entry in &entries {
                    let next_offset = entry.offset().get() as usize;
                    visitor.visit(entry.name(), entry.ino(), entry.type_().into(), next_offset)?;
                    *offset = next_offset;
                }

                Ok(())
            };

            let mut iterate_offset = offset;
            match try_readdir_fn(&mut iterate_offset, visitor) {
                Err(e) if iterate_offset == offset => Err(e),
                // FIXME: FUSE directory offsets are opaque cookies, but this
                // path currently treats them as linear values that can be
                // subtracted to produce a cursor delta.
                _ => Ok(iterate_offset - offset),
            }?
        };

        self.expire_attr_cache();

        Ok(offset_read)
    }

    /// Sets one inode timestamp through `SETATTR`.
    ///
    /// Calls this from VFS timestamp setters whose trait signature cannot
    /// return an error. Failures are logged and the cached metadata is left to be
    /// repaired by a later revalidation.
    pub(super) fn set_time(&self, field: TimeField, time: Duration) {
        let setattr_req = match field {
            TimeField::Access => SetattrReq::new(SetattrValid::empty())
                .set_atime(time.as_secs(), time.subsec_nanos()),
            TimeField::Modify => SetattrReq::new(SetattrValid::empty())
                .set_mtime(time.as_secs(), time.subsec_nanos()),
            TimeField::Change => SetattrReq::new(SetattrValid::empty())
                .set_ctime(time.as_secs(), time.subsec_nanos()),
        };
        if let Err(err) = self.setattr(setattr_req) {
            warn!(
                "virtiofs set_time failed for inode {}: {:?}",
                self.nodeid().as_u64(),
                err
            );
        }
    }

    /// Applies a FUSE `SETATTR` request and commits its returned attributes.
    ///
    /// If the reply loses the attr-version race, only fields selected by the
    /// request's `SetattrValid` mask remain safe to merge into cached metadata.
    pub(super) fn setattr(&self, setattr_req: SetattrReq) -> Result<()> {
        let fs = self.fs_ref();
        let request_attr_version = fs.session().snapshot_attr_version();
        let valid = setattr_req.valid();
        let attr_reply = fs
            .session()
            .do_fuse_op(self.nodeid(), SetattrOperation::new(setattr_req))?;

        self.commit_attr_reply(
            attr_reply,
            request_attr_version,
            StaleAttrAction::MergeSetattr(valid),
        )?;

        Ok(())
    }

    fn forget_async(&self, nlookup: u64) {
        let nodeid = self.nodeid();

        if let Some(fs) = self.fs.upgrade() {
            work_queue::submit_work_func(
                move || {
                    if let Err(err) = fs.session().forget(nodeid, nlookup) {
                        warn!(
                            "virtiofs forget failed for inode {} with nlookup {}: {:?}",
                            nodeid.as_u64(),
                            nlookup,
                            err
                        );
                    }
                },
                WorkPriority::Normal,
            );
        }
    }

    /// Revalidates a cached directory entry with `LOOKUP`.
    ///
    /// The entry TTL and attribute TTL are independent caches. A successful
    /// revalidation refreshes the dentry deadline and commits the returned
    /// attributes as an observation, so a stale attribute reply is discarded.
    pub(super) fn revalidate_lookup(
        &self,
        parent_nodeid: aster_fuse::FuseNodeId,
        name: &str,
    ) -> Result<()> {
        let now = MonotonicCoarseClock::get().read_time();
        if now < *self.entry_valid_until.lock() {
            return Ok(());
        }

        let old_nodeid = self.nodeid();
        let fs = self.fs_ref();
        let request_attr_version = fs.session().snapshot_attr_version();
        let lookup_reply = fs
            .session()
            .do_fuse_op(parent_nodeid, LookupOperation::new(name))?;

        if lookup_reply.nodeid() != old_nodeid || lookup_reply.generation() != self.generation() {
            if let Err(err) = fs.session().forget(lookup_reply.nodeid(), 1) {
                warn!(
                    "virtiofs forget failed for stale lookup inode {}: {:?}",
                    lookup_reply.nodeid().as_u64(),
                    err
                );
            }
            return_errno_with_message!(Errno::ESTALE, "virtiofs stale dentry after revalidate");
        }

        self.commit_entry_reply(
            &lookup_reply,
            request_attr_version,
            StaleAttrAction::Discard,
        )?;

        *self.entry_valid_until.lock() =
            valid_until(lookup_reply.entry_valid(), lookup_reply.entry_valid_nsec());

        Ok(())
    }

    /// Refreshes cached attributes when their server TTL has expired.
    ///
    /// Calls this before operations that depend on a current size or on
    /// page-cache coherency, such as reads, `O_APPEND` writes, and `SEEK_END`.
    /// It is a cheap no-op while the cached attributes are still valid. When a
    /// refresh is needed, the supplied FUSE file handle is passed to `GETATTR`
    /// so the server may return handle-specific attributes.
    pub(in crate::fs::fs_impls::virtiofs) fn revalidate_attr(
        &self,
        fh: FuseFileHandle,
    ) -> Result<()> {
        let now = MonotonicCoarseClock::get().read_time();
        if self.inner.read().is_attr_valid(now) {
            return Ok(());
        }

        let fs = self.fs_ref();
        let request_attr_version = fs.session().snapshot_attr_version();
        let attr_reply = fs.session().do_fuse_op(
            self.nodeid(),
            GetattrOperation::new(GetattrReq::new(GetattrFlags::GETATTR_FH, fh)),
        )?;

        self.commit_attr_reply(attr_reply, request_attr_version, StaleAttrAction::Discard)?;

        Ok(())
    }
}

impl Drop for VirtioFsInode {
    // FUSE forgets must run outside Drop: the session may sleep and
    // may need VFS locks; defer to the work queue.
    fn drop(&mut self) {
        if let Some(fs) = self.fs.upgrade() {
            fs.remove_inode_from_cache(self);
        }

        let nlookup = self.lookup_count.drain();
        if nlookup > 0 {
            self.forget_async(nlookup);
        }
    }
}
