// SPDX-License-Identifier: MPL-2.0

//! Server-issued FUSE open handles for `virtiofs`.

use aster_fuse::{FuseFileHandle, FuseNodeId, FuseOpenFlags, ops::release::ReleaseOptions};

use super::fs::VirtioFs;
use crate::{
    fs::file::{AccessMode, StatusFlags},
    prelude::*,
    thread::work_queue::{self, WorkPriority},
};

/// A server-issued FUSE open handle.
///
/// This object owns `fh` returned by `FUSE_OPEN` or `FUSE_OPENDIR`.
pub(super) struct VirtioFsOpenHandle {
    fh: FuseFileHandle,
    nodeid: FuseNodeId,
    access_mode: AccessMode,
    status_flags: StatusFlags,
    open_flags: FuseOpenFlags,
    fs: Weak<VirtioFs>,
    release_options: ReleaseOptions,
}

impl VirtioFsOpenHandle {
    pub(super) fn new(
        fh: FuseFileHandle,
        nodeid: FuseNodeId,
        access_mode: AccessMode,
        status_flags: StatusFlags,
        open_flags: FuseOpenFlags,
        fs: Weak<VirtioFs>,
        release_options: ReleaseOptions,
    ) -> Arc<Self> {
        Arc::new(Self {
            fh,
            nodeid,
            access_mode,
            status_flags,
            open_flags,
            fs,
            release_options,
        })
    }

    /// Returns the FUSE file handle (`fh`) issued by the server.
    pub(super) fn fh(&self) -> FuseFileHandle {
        self.fh
    }

    /// Returns the composite file flags (access mode | captured status flags)
    /// recorded when this handle was opened.
    pub(super) fn file_flags(&self) -> u32 {
        self.file_flags_with(self.status_flags)
    }

    /// Returns the composite file flags (access mode | status flags) for a
    /// request issued through this handle with the given `status_flags`.
    ///
    /// The access mode is immutable after open, so it is always taken from
    /// this server-issued handle. `status_flags` are per-file-description and
    /// can change via `fcntl` after open, so callers pass the *current* value
    /// (e.g. from `InodeHandle::readdir`) rather than the flags captured at
    /// open time.
    pub(super) fn file_flags_with(&self, status_flags: StatusFlags) -> u32 {
        self.access_mode as u32 | status_flags.bits()
    }

    /// Returns the access mode.
    pub(super) fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    /// Returns the `FUSE_OPEN` reply flags.
    pub(super) fn open_flags(&self) -> FuseOpenFlags {
        self.open_flags
    }
}

impl Drop for VirtioFsOpenHandle {
    fn drop(&mut self) {
        let fs = self.fs.clone();
        let nodeid = self.nodeid;
        let fh = self.fh;
        // `FUSE_RELEASE` uses the flags captured at open time. A handle can be
        // shared across file descriptions or opened transiently (see
        // `OpenHandles`), so a single owning description's current flags are
        // not well-defined; the captured flags are the consistent choice.
        let file_flags = self.file_flags();
        let release_options = self.release_options;

        work_queue::submit_work_func(
            move || {
                // If the filesystem has been unmounted, the release operation is skipped.
                // Reference: <https://github.com/libfuse/libfuse/blob/master/include/fuse_lowlevel.h#L664>.
                let Some(fs) = fs.upgrade() else {
                    return;
                };

                if let Err(err) = fs
                    .session()
                    .release(nodeid, fh, file_flags, release_options)
                {
                    warn!("virtiofs release failed for inode {:?}: {:?}", nodeid, err);
                }
            },
            WorkPriority::Normal,
        );
    }
}

/// Open handles that have been opened on a virtio-fs inode.
pub(super) struct OpenHandles {
    handles: Mutex<Vec<Weak<VirtioFsOpenHandle>>>,
}

impl OpenHandles {
    pub(super) fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }

    /// Registers a handle, pruning dead weak references first.
    pub(super) fn insert(&self, handle: &Arc<VirtioFsOpenHandle>) {
        let mut handles = self.handles.lock();

        handles.retain(|h| h.strong_count() > 0);
        handles.push(Arc::downgrade(handle));
    }

    /// Finds a readable handle, if any.
    pub(super) fn find_readable_handle(&self) -> Option<Arc<VirtioFsOpenHandle>> {
        self.find_handle(AccessMode::is_readable)
    }

    /// Finds a writable handle, if any.
    pub(super) fn find_writable_handle(&self) -> Option<Arc<VirtioFsOpenHandle>> {
        self.find_handle(AccessMode::is_writable)
    }

    fn find_handle(
        &self,
        accepts_fn: impl Fn(&AccessMode) -> bool,
    ) -> Option<Arc<VirtioFsOpenHandle>> {
        let mut handles = self.handles.lock();
        let mut found = None;

        // TODO: Replace this scan-and-fallback scheme with a more direct way
        // to serve inode/page-cache I/O without probing the live open-handle set.
        // Prefer recently inserted handles, which are more likely to have
        // required properties and be valid.
        handles.retain(|handle| {
            let Some(open_handle) = handle.upgrade() else {
                return false;
            };

            if accepts_fn(&open_handle.access_mode) {
                found = Some(open_handle);
            }

            true
        });

        found
    }
}

#[cfg(ktest)]
mod tests {
    use aster_fuse::ops::release::{ReleaseFlags, ReleaseKind};
    use ostd::prelude::ktest;

    use super::*;

    /// Builds a handle whose open-time flag capture is empty, so a test can
    /// tell captured flags apart from live flags passed at request time.
    fn handle_with_empty_capture() -> Arc<VirtioFsOpenHandle> {
        VirtioFsOpenHandle::new(
            FuseFileHandle::new(1),
            FuseNodeId::new(1),
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
            FuseOpenFlags::empty(),
            Weak::new(),
            ReleaseOptions::new(ReleaseKind::Directory, ReleaseFlags::empty()),
        )
    }

    #[ktest]
    fn file_flags_with_composes_live_flags_not_open_time_capture() {
        let handle = handle_with_empty_capture();

        // The capture recorded at open time is empty...
        assert_eq!(handle.file_flags(), AccessMode::O_RDONLY as u32);

        // ...so any extra bits in the composed value must come from the live
        // flags passed in, i.e. from the issuing file description. Regression
        // guard for issue #3536: requests such as `FUSE_READDIR` must carry
        // the current per-open flags, not the open-time capture.
        let live_flags = StatusFlags::O_APPEND | StatusFlags::O_NONBLOCK;
        assert_eq!(
            handle.file_flags_with(live_flags),
            AccessMode::O_RDONLY as u32 | live_flags.bits()
        );

        // Dropping the handle would submit a `FUSE_RELEASE` work item to the
        // global work queue, which is not initialized in the ktest
        // environment. Leak the handle instead; the release is a no-op here
        // anyway since the handle is not connected to a live virtio-fs session.
        core::mem::forget(handle);
    }
}
