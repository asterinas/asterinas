// SPDX-License-Identifier: MPL-2.0

//! Open admission for overlay inodes and the per-open directory handle.
//!
//! A directory open answers with a per-open handle that owns this open file's frozen snapshot and
//! the `..` identity captured here; a writable open of a file runs the writable take point — the
//! read-only gate plus the copy-up promotion — instead. This module is a sibling of
//! [`super::readdir`], so a snapshot's fields stay invisible to the handle: it can read them only
//! through their accessors.

use super::{OverlayInode, readdir::ReaddirCache};
use crate::{
    events::IoEvents,
    fs::{
        file::{
            AccessMode, InodeType, MappableObject, PerOpenFileOps, SettableStatusFlags,
            StatusFlags, SyncMode,
        },
        utils::DirentVisitor,
        vfs::{
            inode::{FileOps, Inode},
            path::Dentry,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    vm::vmar::FileMmapRequest,
};

impl OverlayInode {
    pub(super) fn open_impl(
        &self,
        self_dentry: &Dentry,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        if self.type_().is_directory() {
            // Only this call has the opening dentry, so the `..` identity is captured here.
            let this = self.self_arc();
            return Some(Ok(Box::new(OverlayDirOpenHandle::new(this, self_dentry))));
        }
        if matches!(
            self.type_(),
            InodeType::CharDevice | InodeType::BlockDevice | InodeType::NamedPipe
        ) {
            // A special inode's ops come from its type, not from any layer: the real object's own
            // filesystem opens it, and no copy-up is involved.
            let real = self.real_object();
            return real
                .real_inode()
                .open(real.dentry(), access_mode, status_flags);
        }
        if !access_mode.is_writable() {
            return None;
        }
        match self.writable_real_object(self_dentry) {
            Ok(_) => None,
            Err(err) => Some(Err(err)),
        }
    }
}

/// Holds one open file's own view of an overlay directory: the snapshot it emits from and the `..`
/// it publishes.
struct OverlayDirOpenHandle {
    inode: Arc<OverlayInode>,
    /// Holds this open file's frozen snapshot; `None` until one is taken.
    readdir_cache_snapshot: Mutex<Option<Arc<ReaddirCache>>>,
    /// Holds the `..` inode number captured at open.
    parent_ino: u64,
}

impl OverlayDirOpenHandle {
    pub(super) fn new(inode: Arc<OverlayInode>, self_dentry: &Dentry) -> Self {
        let parent_ino = match self_dentry.parent() {
            Some(parent) => parent.inode().ino(),
            // A parentless dentry is the mount root: `..` degrades to this directory itself.
            None => inode.ino(),
        };
        Self {
            inode,
            readdir_cache_snapshot: Mutex::new(None),
            parent_ino,
        }
    }

    /// Emits this open file's entries from `offset`, resolving the payload it iterates here.
    fn emit_entries(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        // The slot guard spans the whole emission: this handle's slot is the outer lock here.
        let mut slot = self.readdir_cache_snapshot.lock();
        let payload = if offset != 0
            && let Some(frozen) = slot.as_ref()
        {
            Arc::clone(frozen)
        } else {
            let current = self.inode.current_readdir_cache()?; // takes/releases the inode lock inside
            *slot = Some(Arc::clone(&current));
            current
        };
        let mut last_cookie: Option<usize> = None;
        let mut emitted_any = false;
        let mut refusal: Option<Error> = None;
        // The one emission point: it keeps the advance-or-refuse bookkeeping the loop cannot touch.
        let mut emit_item = |name: &str, ino: Option<u64>, type_: InodeType, cookie: usize| {
            if refusal.is_some() {
                return false;
            }
            let Some(ino) = ino else {
                // No inode number means the position is kept while the name is not emitted.
                last_cookie = Some(cookie);
                return true;
            };
            match visitor.visit(name, ino, type_, cookie) {
                Ok(()) => {
                    emitted_any = true;
                    last_cookie = Some(cookie);
                    true
                }
                Err(err) => {
                    refusal = Some(err);
                    false
                }
            }
        };
        if offset < 1 {
            emit_item(".", Some(self.inode.ino()), InodeType::Dir, 1);
        }
        if offset < 2 {
            // The parent identity was captured at open, so the call needs no resolution here.
            emit_item("..", Some(self.parent_ino), InodeType::Dir, 2);
        }
        let start = offset.saturating_sub(2); // cookie 3 + i > offset <=> i >= offset - 2
        for (position, entry) in payload.entries().iter().enumerate().skip(start) {
            let cookie = 3 + position;
            let ino = if entry.is_whiteout() {
                None
            } else if entry.overlay_ino() == 0 {
                match self.inode.lookup(entry.name()) {
                    Ok(child) => Some(child.ino()),
                    // A name that is gone or now hidden stops being visible here as well.
                    Err(err) if err.error() == Errno::ENOENT => None,
                    Err(err) => return Err(err),
                }
            } else {
                Some(entry.overlay_ino())
            };
            if !emit_item(entry.name(), ino, entry.type_(), cookie) {
                break;
            }
        }
        match refusal {
            // A refusal with nothing emitted is the visitor's own refusal, e.g. a small buffer.
            Some(err) if !emitted_any => Err(err),
            _ => Ok(last_cookie.map_or(0, |last| last.saturating_sub(offset))),
        }
    }
}

impl Pollable for OverlayDirOpenHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // A directory is always ready for the next attempt, as it was before it had a handle.
        (IoEvents::IN | IoEvents::OUT) & mask
    }
}

impl FileOps for OverlayDirOpenHandle {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.emit_entries(offset, visitor)
    }
}

impl PerOpenFileOps for OverlayDirOpenHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    // The five below keep what `InodeHandle` reported for a directory without a per-open object.
    fn seek_end(&self) -> Result<Option<usize>> {
        Ok(self.inode.seek_end())
    }

    fn mappable(&self, _request: FileMmapRequest) -> Result<MappableObject<'_>> {
        return_errno_with_message!(Errno::ENODEV, "the file is not mappable");
    }

    fn sync(&self, mode: SyncMode) -> Result<()> {
        self.inode.sync(mode)
    }

    fn settable_status_flags(&self) -> SettableStatusFlags {
        SettableStatusFlags::minimal()
    }
}
