// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use aster_block::bio::BioWaiter;
use aster_virtio::device::filesystem::{
    device::VirtioFsDirEntry,
    protocol::{
        FATTR_ATIME, FATTR_CTIME, FATTR_GID, FATTR_MODE, FATTR_MTIME, FATTR_SIZE, FATTR_UID,
        FOPEN_DIRECT_IO, FOPEN_KEEP_CACHE, FuseAttrOut, SetattrIn,
    },
};
use log::warn;
use ostd::{
    mm::{HasSize, VmReader, VmWriter, io::util::HasVmReaderWriter},
    sync::RwLock,
};

use super::{FUSE_READDIR_BUF_SIZE, VirtioFs, handle::VirtioFsHandle, valid_duration};
use crate::{
    fs::{
        file::{AccessMode, FileIo, InodeMode, InodeType, StatusFlags},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode, InodeIo, Metadata, SymbolicLink},
            page_cache::{CachePage, PageCache, PageCacheBackend},
            path::Dentry,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    thread::work_queue::{WorkPriority, submit_work_func},
    time::clocks::MonotonicCoarseClock,
    vm::vmo::Vmo,
};

pub(super) struct VirtioFsInode {
    this: Weak<VirtioFsInode>,
    nodeid: AtomicU64,
    lookup_count: AtomicU64,
    metadata: RwLock<Metadata>,
    // TODO: Move the entry timeout state to `Dentry` once the VFS can carry
    // filesystem-specific per-dentry data. This timeout belongs to the cached
    // name-to-inode association, not to the inode object itself.
    // Reference: https://codebrowser.dev/linux/linux/fs/fuse/dir.c.html#98
    // Reference: https://codebrowser.dev/linux/linux/fs/fuse/dir.c.html#275
    entry_valid_until: RwLock<Option<Duration>>,
    attr_valid_until: RwLock<Duration>,
    page_cache: Option<Mutex<PageCache>>,
    page_cache_fh: Mutex<Option<u64>>,
    fs: Weak<VirtioFs>,
    extension: Extension,
}

impl VirtioFsInode {
    pub(super) fn new(
        nodeid: u64,
        metadata: Metadata,
        fs: Weak<VirtioFs>,
        entry_valid_until: Option<Duration>,
        attr_valid_until: Duration,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            this: weak_self.clone(),
            nodeid: AtomicU64::new(nodeid),
            lookup_count: AtomicU64::new(0),
            metadata: RwLock::new(metadata),
            entry_valid_until: RwLock::new(entry_valid_until),
            attr_valid_until: RwLock::new(attr_valid_until),
            page_cache: metadata.type_.is_regular_file().then(|| {
                Mutex::new(PageCache::with_capacity(metadata.size, weak_self.clone() as _).unwrap())
            }),
            page_cache_fh: Mutex::new(None),
            fs,
            extension: Extension::new(),
        })
    }

    pub(super) fn fs_ref(&self) -> Arc<VirtioFs> {
        self.fs.upgrade().unwrap()
    }

    pub(super) fn try_fs_ref(&self) -> Option<Arc<VirtioFs>> {
        self.fs.upgrade()
    }

    pub(super) fn nodeid(&self) -> u64 {
        self.nodeid.load(Ordering::Relaxed)
    }

    pub(super) fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn increase_lookup_count(&self) {
        self.lookup_count.fetch_add(1, Ordering::Relaxed);
    }

    fn release_lookup_count(&self, nlookup: u64) {
        if nlookup == 0 {
            return;
        }

        self.forget_async(nlookup);
    }

    fn forget_async(&self, nlookup: u64) {
        let nodeid = self.nodeid();

        if let Some(fs) = self.fs.upgrade() {
            submit_work_func(
                move || {
                    let _ = fs.device.fuse_forget(nodeid, nlookup);
                },
                WorkPriority::Normal,
            );
        }
    }

    fn get_or_open_page_cache_fh(&self) -> Result<u64> {
        let mut fh_slot = self.page_cache_fh.lock();
        if let Some(fh) = *fh_slot {
            return Ok(fh);
        }

        let fs = self.fs_ref();
        let open_out = fs
            .device
            .fuse_open(self.nodeid(), AccessMode::O_RDWR.into())
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs page cache open failed"))?;
        *fh_slot = Some(open_out.fh);
        Ok(open_out.fh)
    }

    fn release_page_cache_fh(&self) {
        let Some(fs) = self.fs.upgrade() else {
            return;
        };

        if let Some(fh) = self.page_cache_fh.lock().take() {
            let _ = fs
                .device
                .fuse_release(self.nodeid(), fh, AccessMode::O_RDWR.into());
        }
    }

    fn revalidate_lookup(&self, parent_nodeid: u64, name: &str) -> Result<()> {
        let now = MonotonicCoarseClock::get().read_time();
        if self
            .entry_valid_until
            .read()
            .is_none_or(|valid_until| now < valid_until)
        {
            return Ok(());
        }

        let old_nodeid = self.nodeid();
        let fs = self.fs_ref();
        let entry_out = fs
            .device
            .fuse_lookup(parent_nodeid, name)
            .map_err(Error::from)?;

        if entry_out.nodeid != old_nodeid {
            // The returned entry refers to a different inode. Drop the lookup
            // reference immediately so we don't leak nlookup on that node.
            let _ = fs.device.fuse_forget(entry_out.nodeid, 1);
            return_errno_with_message!(Errno::ESTALE, "virtiofs stale dentry after revalidate");
        }

        // Count only lookups that still point to this inode.
        self.increase_lookup_count();

        *self.metadata.write() = Metadata::from(entry_out.attr);

        let now = MonotonicCoarseClock::get().read_time();
        *self.entry_valid_until.write() = Some(now.saturating_add(valid_duration(
            entry_out.entry_valid,
            entry_out.entry_valid_nsec,
        )));
        *self.attr_valid_until.write() = now.saturating_add(valid_duration(
            entry_out.attr_valid,
            entry_out.attr_valid_nsec,
        ));

        Ok(())
    }

    pub(super) fn revalidate_attr(&self) -> Result<()> {
        let now = MonotonicCoarseClock::get().read_time();
        if now < *self.attr_valid_until.read() {
            return Ok(());
        }

        let old_metadata = self.metadata();
        let fs = self.fs_ref();
        let attr_out = fs.device.fuse_getattr(self.nodeid()).map_err(Error::from)?;

        let new_metadata = Metadata::from(attr_out.attr);
        if old_metadata.last_modify_at != new_metadata.last_modify_at {
            self.invalidate_page_cache(new_metadata.size)?;
        }
        *self.metadata.write() = new_metadata;

        let now = MonotonicCoarseClock::get().read_time();
        *self.attr_valid_until.write() = now.saturating_add(valid_duration(
            attr_out.attr_valid,
            attr_out.attr_valid_nsec,
        ));

        Ok(())
    }

    fn invalidate_page_cache(&self, new_size: usize) -> Result<()> {
        let Some(page_cache) = &self.page_cache else {
            return Ok(());
        };

        let page_cache = &mut page_cache.lock();

        let cached_size = page_cache.pages().size();
        if cached_size > 0 {
            // Dirty cache pages are laundered before they are removed from the page cache,
            // instead of being silently dropped.
            // Reference: https://codebrowser.dev/linux/linux/fs/fuse/file.c.html#292
            // Reference: https://codebrowser.dev/linux/linux/mm/truncate.c.html#633
            page_cache.evict_range(0..cached_size)?;
            page_cache.resize(0)?;
        }
        page_cache.resize(new_size)?;

        Ok(())
    }

    pub(super) fn flush_page_cache(&self) -> Result<()> {
        let Some(page_cache) = &self.page_cache else {
            return Ok(());
        };

        page_cache.lock().evict_range(0..self.size())?;
        Ok(())
    }

    pub(super) fn cached_read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        fh: u64,
    ) -> Result<usize> {
        self.revalidate_attr()?;

        let Some(page_cache) = &self.page_cache else {
            return self.direct_read_at(offset, writer, fh);
        };

        let file_size = self.size();
        let start = file_size.min(offset);
        let end = file_size.min(offset + writer.avail());
        let read_len = end - start;
        page_cache.lock().pages().read(start, writer)?;
        Ok(read_len)
    }

    pub(super) fn direct_read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        fh: u64,
    ) -> Result<usize> {
        self.revalidate_attr()?;

        let file_size = self.size();
        let start = file_size.min(offset);
        let end = file_size.min(offset + writer.avail());
        let read_len = end - start;
        let max_len = read_len.min(u32::MAX as usize) as u32;
        let data = self
            .fs_ref()
            .device
            .fuse_read(self.nodeid(), fh, start as u64, max_len)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs read failed"))?;
        let mut reader = VmReader::from(data.as_slice());
        writer.write_fallible(&mut reader)?;
        Ok(read_len.min(data.len()))
    }

    pub(super) fn cached_write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        fh: u64,
    ) -> Result<usize> {
        let Some(page_cache) = &self.page_cache else {
            return self.direct_write_at(offset, reader, fh);
        };
        let page_cache = page_cache.lock();

        let write_len = reader.remain();
        let new_size = offset + write_len;
        if new_size > page_cache.pages().size() {
            page_cache.resize(new_size)?;
        }
        {
            let mut metadata = self.metadata.write();
            metadata.size = metadata.size.max(new_size);
            metadata.nr_sectors_allocated = metadata.size.div_ceil(512);
        }
        page_cache.pages().write(offset, reader)?;

        Ok(write_len)
    }

    pub(super) fn direct_write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        fh: u64,
    ) -> Result<usize> {
        let write_len = reader.remain().min(u32::MAX as usize);
        let mut data = vec![0u8; write_len];
        reader.read_fallible(&mut VmWriter::from(data.as_mut_slice()))?;

        let written = self
            .fs_ref()
            .device
            .fuse_write(self.nodeid(), fh, offset as u64, &data)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs write failed"))?;

        let new_size = offset + written;
        {
            let mut metadata = self.metadata.write();
            metadata.size = metadata.size.max(new_size);
            metadata.nr_sectors_allocated = metadata.size.div_ceil(512);
        }

        self.invalidate_page_cache(self.size())?;

        Ok(written)
    }

    fn open_handle(&self, access_mode: AccessMode) -> Result<VirtioFsHandle> {
        let flags = u32::from(access_mode);

        let fs = self.fs_ref();
        let open_out = fs
            .device
            .fuse_open(self.nodeid(), flags)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs open failed"))?;
        let cache_enabled =
            self.page_cache.is_some() && (open_out.open_flags & FOPEN_DIRECT_IO == 0);

        if open_out.open_flags & FOPEN_KEEP_CACHE == 0 {
            self.invalidate_page_cache(self.size())?;
        }

        let Some(inode) = self.this.upgrade() else {
            let _ = fs.device.fuse_release(self.nodeid(), open_out.fh, flags);
            return_errno_with_message!(Errno::EIO, "virtiofs inode is unavailable");
        };

        Ok(VirtioFsHandle::new(
            inode,
            open_out.fh,
            flags,
            cache_enabled,
        ))
    }

    fn setattr(&self, setattr_in: SetattrIn) -> Result<()> {
        let fs = self.fs_ref();
        let attr_out = fs
            .device
            .fuse_setattr(self.nodeid(), setattr_in)
            .map_err(Error::from)?;

        let old_metadata = self.metadata();
        let new_metadata = Metadata::from(attr_out.attr);
        if old_metadata.last_modify_at != new_metadata.last_modify_at {
            self.invalidate_page_cache(new_metadata.size)?;
        }
        *self.metadata.write() = new_metadata;

        let now = MonotonicCoarseClock::get().read_time();
        *self.attr_valid_until.write() = now.saturating_add(valid_duration(
            attr_out.attr_valid,
            attr_out.attr_valid_nsec,
        ));

        Ok(())
    }
}

impl Drop for VirtioFsInode {
    fn drop(&mut self) {
        self.release_page_cache_fh();
        let nlookup = self.lookup_count.load(Ordering::Relaxed);
        self.release_lookup_count(nlookup);
    }
}

impl PageCacheBackend for VirtioFsInode {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let offset = idx * PAGE_SIZE;
        if offset >= self.size() {
            return_errno_with_message!(Errno::EINVAL, "virtiofs page read beyond EOF");
        }

        frame.writer().fill_zeros(frame.size());
        let size = (self.size() - offset).min(PAGE_SIZE).min(u32::MAX as usize) as u32;
        let fs = self.fs_ref();
        let fh = self.get_or_open_page_cache_fh()?;
        let data = fs
            .device
            .fuse_read(self.nodeid(), fh, offset as u64, size)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs page read failed"))?;
        let mut frame_writer = frame.writer();
        frame_writer.write_fallible(&mut VmReader::from(data.as_slice()).to_fallible())?;
        Ok(BioWaiter::new())
    }

    fn write_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let offset = idx * PAGE_SIZE;
        let file_size = self.size();
        if offset >= file_size {
            return Ok(BioWaiter::new());
        }

        let write_len = (file_size - offset).min(PAGE_SIZE);
        let mut data = vec![0u8; write_len];
        let mut writer = VmWriter::from(data.as_mut_slice());
        writer.write_fallible(&mut frame.reader().to_fallible())?;

        let fs = self.fs_ref();
        let fh = self.get_or_open_page_cache_fh()?;
        fs.device
            .fuse_write(self.nodeid(), fh, offset as u64, &data)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs page write failed"))?;
        Ok(BioWaiter::new())
    }

    fn npages(&self) -> usize {
        self.size().div_ceil(PAGE_SIZE)
    }
}

impl InodeIo for VirtioFsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "inode is not a regular file");
        }

        let fs = self.fs_ref();
        let fh = fs
            .device
            .fuse_open(self.nodeid(), AccessMode::O_RDONLY.into())
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs read failed"))?
            .fh;
        let result = self.cached_read_at(offset, writer, fh);
        let _ = fs
            .device
            .fuse_release(self.nodeid(), fh, AccessMode::O_RDONLY.into());
        result
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "inode is not a regular file");
        }

        let offset = if status_flags.contains(StatusFlags::O_APPEND) {
            self.revalidate_attr()?;
            self.size()
        } else {
            offset
        };

        let fs = self.fs_ref();
        let fh = fs
            .device
            .fuse_open(self.nodeid(), AccessMode::O_WRONLY.into())
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs write failed"))?
            .fh;
        let result = self.cached_write_at(offset, reader, fh);
        let _ = fs
            .device
            .fuse_release(self.nodeid(), fh, AccessMode::O_WRONLY.into());
        result
    }
}

impl Inode for VirtioFsInode {
    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.metadata.read().type_ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "resize on non-regular file");
        }

        let size = u64::try_from(new_size)
            .map_err(|_| Error::with_message(Errno::EFBIG, "virtiofs resize size too large"))?;

        let setattr_in = SetattrIn {
            valid: FATTR_SIZE,
            size,
            ..SetattrIn::default()
        };
        self.setattr(setattr_in)
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn ino(&self) -> u64 {
        self.nodeid()
    }

    fn type_(&self) -> InodeType {
        self.metadata.read().type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let mode_bits = u32::from(self.type_()) | u32::from(mode.bits());
        let setattr_in = SetattrIn {
            valid: FATTR_MODE,
            mode: mode_bits,
            ..SetattrIn::default()
        };
        self.setattr(setattr_in)
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let setattr_in = SetattrIn {
            valid: FATTR_UID,
            uid: uid.into(),
            ..SetattrIn::default()
        };
        self.setattr(setattr_in)
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let setattr_in = SetattrIn {
            valid: FATTR_GID,
            gid: gid.into(),
            ..SetattrIn::default()
        };
        self.setattr(setattr_in)
    }

    fn atime(&self) -> Duration {
        self.metadata.read().last_access_at
    }

    fn set_atime(&self, time: Duration) {
        let setattr_in = SetattrIn {
            valid: FATTR_ATIME,
            atime: time.as_secs(),
            atimensec: time.subsec_nanos(),
            ..SetattrIn::default()
        };
        if let Err(err) = self.setattr(setattr_in) {
            warn!(
                "virtiofs set_atime failed for inode {}: {:?}",
                self.nodeid(),
                err
            );
        }
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().last_modify_at
    }

    fn set_mtime(&self, time: Duration) {
        let setattr_in = SetattrIn {
            valid: FATTR_MTIME,
            mtime: time.as_secs(),
            mtimensec: time.subsec_nanos(),
            ..SetattrIn::default()
        };
        if let Err(err) = self.setattr(setattr_in) {
            warn!(
                "virtiofs set_mtime failed for inode {}: {:?}",
                self.nodeid(),
                err
            );
        }
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().last_meta_change_at
    }

    fn set_ctime(&self, time: Duration) {
        let setattr_in = SetattrIn {
            valid: FATTR_CTIME,
            ctime: time.as_secs(),
            ctimensec: time.subsec_nanos(),
            ..SetattrIn::default()
        };
        if let Err(err) = self.setattr(setattr_in) {
            warn!(
                "virtiofs set_ctime failed for inode {}: {:?}",
                self.nodeid(),
                err
            );
        }
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.page_cache
            .as_ref()
            .map(|page_cache| page_cache.lock().pages().clone())
    }

    fn open(
        &self,
        access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        if self.metadata.read().type_ != InodeType::File {
            return None;
        }
        Some(
            self.open_handle(access_mode)
                .map(|handle| Box::new(handle) as Box<dyn FileIo>),
        )
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "lookup on non-directory")
        }

        let fs = self.fs_ref();
        let parent_nodeid = self.nodeid();
        let entry_out = fs
            .device
            .fuse_lookup(parent_nodeid, name)
            .map_err(Error::from)?;
        let nodeid = entry_out.nodeid;

        let now = MonotonicCoarseClock::get().read_time();

        let entry_valid_until = Some(now.saturating_add(valid_duration(
            entry_out.entry_valid,
            entry_out.entry_valid_nsec,
        )));
        let attr_valid_until = now.saturating_add(valid_duration(
            entry_out.attr_valid,
            entry_out.attr_valid_nsec,
        ));

        let inode = VirtioFsInode::new(
            nodeid,
            Metadata::from(entry_out.attr),
            Arc::downgrade(&fs),
            entry_valid_until,
            attr_valid_until,
        );
        inode.increase_lookup_count();

        Ok(inode)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "create on non-directory")
        }

        let fs = self.fs_ref();
        let parent_nodeid = self.nodeid();
        let (entry_out, open_out_opt) = match type_ {
            InodeType::File => {
                let (entry_out, open_out) = fs
                    .device
                    .fuse_create(
                        parent_nodeid,
                        name,
                        u32::from(InodeType::File) | u32::from(mode.bits()),
                    )
                    .map_err(Error::from)?;
                (entry_out, Some(open_out))
            }
            InodeType::Dir => {
                let entry_out = fs
                    .device
                    .fuse_mkdir(
                        parent_nodeid,
                        name,
                        u32::from(InodeType::Dir) | u32::from(mode.bits()),
                    )
                    .map_err(Error::from)?;
                (entry_out, None)
            }
            InodeType::Socket => {
                let entry_out = fs
                    .device
                    .fuse_mknod(
                        parent_nodeid,
                        name,
                        u32::from(InodeType::Socket) | u32::from(mode.bits()),
                        0,
                    )
                    .map_err(Error::from)?;
                (entry_out, None)
            }
            _ => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "virtiofs create supports file/dir/socket only"
                )
            }
        };
        let attr_out: FuseAttrOut = fs
            .device
            .fuse_getattr(entry_out.nodeid)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs getattr after create failed"))?;

        if let Some(open_out) = open_out_opt {
            let _ =
                fs.device
                    .fuse_release(entry_out.nodeid, open_out.fh, AccessMode::O_RDWR.into());
        }

        let now = MonotonicCoarseClock::get().read_time();

        let entry_valid_until = Some(now.saturating_add(valid_duration(
            entry_out.entry_valid,
            entry_out.entry_valid_nsec,
        )));
        let attr_valid_until = now.saturating_add(valid_duration(
            attr_out.attr_valid,
            attr_out.attr_valid_nsec,
        ));

        let inode = VirtioFsInode::new(
            entry_out.nodeid,
            Metadata::from(attr_out.attr),
            Arc::downgrade(&fs),
            entry_valid_until,
            attr_valid_until,
        );
        inode.increase_lookup_count();

        Ok(inode)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "link on non-directory")
        }

        let old = old
            .downcast_ref::<VirtioFsInode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;

        let fs = self.fs_ref();
        fs.device
            .fuse_link(old.nodeid(), self.nodeid(), name)
            .map_err(Error::from)?;
        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "unlink on non-directory")
        }

        let fs = self.fs_ref();
        fs.device
            .fuse_unlink(self.nodeid(), name)
            .map_err(Error::from)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "rmdir on non-directory")
        }

        let fs = self.fs_ref();
        fs.device
            .fuse_rmdir(self.nodeid(), name)
            .map_err(Error::from)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "readdir on non-directory")
        }

        let fs = self.fs_ref();
        let entries: Vec<VirtioFsDirEntry> = fs
            .device
            .fuse_opendir(self.nodeid())
            .and_then(|fh| {
                let result =
                    fs.device
                        .fuse_readdir(self.nodeid(), fh, offset as u64, FUSE_READDIR_BUF_SIZE);
                let _ = fs.device.fuse_releasedir(self.nodeid(), fh);
                result
            })
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs readdir failed"))?;

        let mut current_off = offset;
        for entry in &entries {
            current_off = entry.offset as usize;
            visitor.visit(
                entry.name.as_str(),
                entry.ino,
                inode_type_from_dirent_type(entry.type_),
                current_off,
            )?;
        }

        Ok(current_off)
    }

    fn sync_data(&self) -> Result<()> {
        self.flush_page_cache()
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs_ref()
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        let metadata = self.metadata();
        if metadata.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "read_link on non-symlink")
        }

        let fs = self.fs_ref();
        let target = fs
            .device
            .fuse_readlink(self.nodeid())
            .map_err(Error::from)?;

        Ok(SymbolicLink::Plain(target))
    }

    fn revalidate_child(&self, name: &str, child: &Dentry) -> Result<()> {
        let Some(parent) = child.parent() else {
            return Ok(());
        };

        self.revalidate_lookup(parent.inode().ino(), name)
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

fn inode_type_from_dirent_type(type_: u32) -> InodeType {
    match type_ {
        4 => InodeType::Dir,
        8 => InodeType::File,
        10 => InodeType::SymLink,
        2 => InodeType::CharDevice,
        6 => InodeType::BlockDevice,
        1 => InodeType::NamedPipe,
        12 => InodeType::Socket,
        _ => InodeType::Unknown,
    }
}
