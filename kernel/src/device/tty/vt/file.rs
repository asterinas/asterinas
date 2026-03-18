// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use inherit_methods_macro::inherit_methods;

use crate::{
    device::tty::{
        Tty,
        vt::{
            VtDriver,
            manager::{VT_MANAGER, VtManagerGuard},
        },
    },
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

/// Open file counter of a VT.
pub(super) struct VtOpenFileCounter(AtomicUsize);

impl VtOpenFileCounter {
    pub(super) const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// Increments the open-file counter.
    ///
    /// The `_guard` argument makes holding the VT manager lock a type-level
    /// requirement for increments. Since `has_open_files` also requires the
    /// same lock, a `false` result from `has_open_files` is stable against
    /// concurrent opens while the guard is held.
    fn acquire_one(&self, _guard: &VtManagerGuard<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn release_one(&self) {
        let old_count = self.0.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old_count > 0, "releasing more files than acquired");
    }

    /// Returns whether this VT currently has open files.
    ///
    /// The `_guard` argument makes this observation happen under the VT
    /// manager lock. If this returns `false`, the caller can rely on the VT
    /// having no open files while holding the guard, because new opens must
    /// acquire the same lock before incrementing the counter. A `true` result
    /// is only a snapshot: files may be closed concurrently, so the counter
    /// may become zero immediately after this method returns.
    pub(super) fn has_open_files(&self, _guard: &VtManagerGuard<'_>) -> bool {
        self.0.load(Ordering::Relaxed) > 0
    }
}

/// The file representation of a virtual terminal (VT) device.
pub(super) struct VtFile(Arc<Tty<VtDriver>>);

impl VtFile {
    pub(super) fn new(tty: Arc<Tty<VtDriver>>) -> Result<VtFile> {
        let driver = tty.driver();
        let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
        let guard = vtm.lock();
        guard.allocate_vt(driver.index());
        driver.open_file_counter().acquire_one(&guard);

        Ok(VtFile(tty))
    }
}

impl Drop for VtFile {
    fn drop(&mut self) {
        self.0.driver().open_file_counter().release_one();
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for VtFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl FileOps for VtFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.write(reader, status_flags)
    }
}

#[inherit_methods(from = "self.0")]
impl PerOpenFileOps for VtFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
