// SPDX-License-Identifier: MPL-2.0

//! LSM attributes under `/proc/<pid>/attr`.

use super::TidDirOps;
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{
            ListedEntry, ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
            visit_listed_entries,
        },
        vfs::inode::Inode,
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    security::lsm,
    thread::Thread,
};

/// Represents `/proc/<pid>/attr`.
pub struct AttrDirOps(TidDirOps);

impl AttrDirOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self(dir.clone()), parent, mkmod!(a+rx))
    }
}

impl ProcDirOps for AttrDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if name != "current" {
            return_errno_with_message!(Errno::ENOENT, "the LSM attribute does not exist");
        }

        Ok(CurrentFileOps::new_inode(
            self.0.clone(),
            this_dir.this_weak().clone(),
        ))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            [ListedEntry::new("current", InodeType::File)],
            visit_fn,
        )
    }
}

/// Represents `/proc/<pid>/attr/current`.
struct CurrentFileOps(TidDirOps);

impl CurrentFileOps {
    fn new_inode(dir: TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for CurrentFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self
            .0
            .thread()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the thread does not exist"))?;
        let posix_thread = thread.as_posix_thread().unwrap();
        let mut value = lsm::task_attr_current(posix_thread)?;
        value.push('\n');

        let bytes = value.as_bytes();
        let mut reader = VmReader::from(&bytes[offset.min(bytes.len())..]);
        Ok(writer.write_fallible(&mut reader)?)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if offset != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "an LSM task attribute must be written at offset zero"
            );
        }

        let target = self
            .0
            .thread()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the thread does not exist"))?;
        let current = Thread::current()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "there is no current thread"))?;
        if !Arc::ptr_eq(&target, &current) {
            return_errno_with_message!(
                Errno::EPERM,
                "an LSM task attribute can only be changed by the current thread"
            );
        }

        let input_len = reader.remain();
        // Keep the generic procfs buffer bounded. Each LSM validates the
        // syntax and semantic length of its own task attribute.
        if input_len == 0 || input_len > PAGE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "the LSM task attribute has an invalid size");
        }

        let (value, bytes_read) = reader.read_cstring_until_end(input_len)?;
        if bytes_read != input_len || value.as_bytes().len() != input_len {
            return_errno_with_message!(Errno::EINVAL, "the LSM task attribute contains a NUL byte");
        }
        let value = value.to_str().map_err(|_| {
            Error::with_message(Errno::EINVAL, "the LSM task attribute is not UTF-8")
        })?;
        lsm::set_task_attr_current(target.as_posix_thread().unwrap(), value)?;

        Ok(bytes_read)
    }
}
