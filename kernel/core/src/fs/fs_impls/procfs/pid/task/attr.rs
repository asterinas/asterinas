// SPDX-License-Identifier: MPL-2.0

//! Task security attributes exposed through procfs.
//!
//! [`AttrDirOps`] locates tasks through [`TidDirOps`].
//! Opening `current` authorizes access and captures the LSM credential state.
//! [`CurrentFileHandle`] retains its formatted snapshot across exec and exit,
//! so later reads cannot retarget a reused numeric PID.

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, InodeType, PerOpenFileOps, StatusFlags, mkmod},
        procfs::template::{
            self, ListedEntry, ProcDir, ProcDirOps, ProcFile, ProcFileOpsByHandle, ReaddirEntry,
        },
        vfs::inode::{FileOps, Inode, RevalidationPolicy},
    },
    prelude::*,
    process::{
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
        signal::{PollHandle, Pollable},
    },
    thread::Thread,
};

/// `/proc/[pid]/attr` and `/proc/[pid]/task/[tid]/attr`.
pub(super) struct AttrDirOps(TidDirOps);

impl AttrDirOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self(dir.clone()), parent, mkmod!(a+rx))
    }
}

impl ProcDirOps for AttrDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.thread().is_none() {
            return_errno!(Errno::ESRCH);
        }
        if name != "current" {
            return_errno!(Errno::ENOENT);
        }
        Ok(ProcFile::new(
            CurrentFileOps(self.0.clone()),
            this_dir.this_weak().clone(),
            mkmod!(a+r),
        ))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.0.thread().is_none() {
            return_errno!(Errno::ESRCH);
        }
        template::visit_listed_entries(
            offset,
            [ListedEntry::new("current", InodeType::File)],
            visit_fn,
        )
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        self.0.thread().is_some()
    }
}

struct CurrentFileOps(TidDirOps);

impl ProcFileOpsByHandle for CurrentFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn open(&self, _access: AccessMode, _flags: StatusFlags) -> Result<Box<dyn PerOpenFileOps>> {
        let process = self.0.process().ok_or_else(|| Error::new(Errno::ESRCH))?;
        // Lock order: process VMAR -> PID entry -> credential state.
        // Exec publishes the image and credentials under the same VMAR lock.
        let vmar = process.lock_vmar();
        let thread = self.0.thread().ok_or_else(|| Error::new(Errno::ESRCH))?;
        let target = thread.as_posix_thread().unwrap();
        if vmar.as_ref().is_none()
            || thread.is_exited()
            || !Arc::ptr_eq(&target.process(), &process)
        {
            return_errno!(Errno::ESRCH);
        }
        target
            .check_alien_access_from(
                current_thread!().as_posix_thread().unwrap(),
                AlienAccessMode::READ_WITH_FS_CREDS,
            )
            .map_err(|_| Error::new(Errno::EACCES))?;
        let state = target.credentials().security();
        drop(vmar);

        // Retain only the authorized snapshot: later reads cannot retarget a PID,
        // or mix the old and new label when exec races with a partial read.
        let value = state.current_attr()?;
        Ok(Box::try_new(CurrentFileHandle(value)).map_err(|_| Error::new(Errno::ENOMEM))?)
    }
}

struct CurrentFileHandle(String);

impl Pollable for CurrentFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT) & mask
    }
}

impl FileOps for CurrentFileHandle {
    fn read_at(&self, offset: usize, writer: &mut VmWriter, _flags: StatusFlags) -> Result<usize> {
        let bytes = self.0.as_bytes();
        Ok(writer.write_fallible(&mut VmReader::from(&bytes[offset.min(bytes.len())..]))?)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _flags: StatusFlags,
    ) -> Result<usize> {
        return_errno!(Errno::EPERM);
    }
}

impl PerOpenFileOps for CurrentFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }
    fn is_offset_aware(&self) -> bool {
        true
    }
}
