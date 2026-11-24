// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{
            self, ListedEntry, ProcDir, ProcDirOps, ProcFile, ProcFileOps, ReaddirEntry,
        },
        vfs::inode::{Inode, RevalidationPolicy},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    security::lsm::proc_attr,
    thread::Thread,
};

/// Represents a task's `attr` directory or a module-specific directory beneath it.
#[derive(Clone)]
pub(super) struct AttrDirOps {
    dir: TidDirOps,
    module: Option<&'static str>,
}

impl AttrDirOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(
            Self {
                dir: dir.clone(),
                module: None,
            },
            parent,
            mkmod!(a+rx),
        )
    }
}

impl ProcDirOps for AttrDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.dir.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.dir.pid_entry().type_().is_none() {
            return Err(errno!(ENOENT, "the task does not exist"));
        }

        if name == "current" {
            return Ok(ProcFile::new(
                CurrentAttrFileOps(self.clone()),
                this_dir.this_weak().clone(),
                mkmod!(a+r),
            ));
        }

        if self.module.is_none()
            && let Some(module) = proc_attr::module_names().find(|module| *module == name)
        {
            return Ok(ProcDir::new(
                Self {
                    dir: self.dir.clone(),
                    module: Some(module),
                },
                this_dir.this_weak().clone(),
                mkmod!(a+rx),
            ));
        }

        Err(errno!(ENOENT, "the attribute does not exist"))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.dir.pid_entry().type_().is_none() {
            return Err(errno!(ENOENT, "the task does not exist"));
        }

        let entries = core::iter::once(ListedEntry::new("current", InodeType::File)).chain(
            proc_attr::module_names()
                .filter(|_| self.module.is_none())
                .map(|name| ListedEntry::new(name, InodeType::Dir)),
        );
        template::visit_listed_entries(offset, entries, visit_fn)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        self.dir.pid_entry().type_().is_some()
    }
}

/// Represents a task's read-only `current` security attribute.
struct CurrentAttrFileOps(AttrDirOps);

impl ProcFileOps for CurrentAttrFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.dir.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self
            .0
            .dir
            .thread()
            .ok_or_else(|| errno!(ESRCH, "the task does not exist"))?;
        let value = proc_attr::read_current(self.0.module, thread.as_posix_thread().unwrap())?;
        let bytes = value.as_bytes();
        let bytes_read =
            writer.write_fallible(&mut VmReader::from(&bytes[offset.min(bytes.len())..]))?;
        Ok(bytes_read)
    }
}
