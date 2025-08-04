// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps},
        utils::{DirEntryVecExt, Inode},
    },
    namespace::{NameSpace, NsFile},
    prelude::*,
    process::{posix_thread::AsPosixThread, Process},
};

/// Represents the inode at `/proc/[pid]/ns`.
pub struct NsDirOps(Arc<Process>);

impl NsDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self(process_ref.clone()))
            .parent(parent)
            .volatile()
            .build()
            .unwrap()
    }
}

impl DirOps for NsDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let main_thread = self.0.main_thread();

        let ns_context = {
            let posix_thread = main_thread.as_posix_thread().unwrap();
            posix_thread.ns_context().lock()
        };

        let ns_context_locked = ns_context
            .as_ref()
            .ok_or_else(|| {
                Error::with_message(Errno::ENOENT, "the process does not have namespaces")
            })?
            .read();

        // Mode 1: Look up by namespace name (e.g., "user").
        // This returns a symbolic link to the namespace.
        if let Some(ns) = ns_context_locked.iter_ns().find_map(|ns| {
            if ns.name() == name {
                ns.weak_self().upgrade()
            } else {
                None
            }
        }) {
            return Ok(NsSymOps::new_inode(ns, this_ptr));
        };

        // Mode 2: Look up by symlink content.
        // This returns an `NsFile` directly, as if the symlink had been resolved.
        //
        // FIXME: The following lookup mode implements non-standard behavior.
        // It allows resolving a namespace by its symlink content (e.g., "user:[4026531837]")
        // in a single lookup. This is necessary for internal kernel operations that
        // resolve a link and open the file in one step. This should ideally be disallowed
        // for direct userspace lookups, but we currently lack the mechanism to
        // differentiate the caller.
        if let Some(ns) = ns_context_locked.iter_ns().find_map(|ns| {
            if ns.proc_symlink().as_str() == name {
                ns.weak_self().upgrade()
            } else {
                None
            }
        }) {
            return Ok(Arc::new(NsFile::new(ns)));
        }

        return_errno_with_message!(Errno::ENOENT, "the process does not have namespaces");
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let main_thread = self.0.main_thread();

        let ns_context = {
            let posix_thread = main_thread.as_posix_thread().unwrap();
            posix_thread.ns_context().lock()
        };

        let Some(ns_context) = ns_context.as_ref() else {
            return;
        };

        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<NsDirOps>>().unwrap().this()
        };

        let mut cached_children = this.cached_children().write();
        ns_context.read().iter_ns().for_each(|ns| {
            let Some(ns) = ns.weak_self().upgrade() else {
                return;
            };
            cached_children.put_entry_if_not_found(ns.name(), || {
                NsSymOps::new_inode(ns.clone(), this_ptr.clone())
            });
        });
    }
}

/// Represents the inode at `/proc/[pid]/ns/N`.
struct NsSymOps(Arc<dyn NameSpace>);

impl NsSymOps {
    pub fn new_inode(ns: Arc<dyn NameSpace>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(Self(ns))
            .parent(parent)
            .volatile()
            .build()
            .unwrap()
    }
}

impl SymOps for NsSymOps {
    fn read_link(&self) -> Result<String> {
        Ok(self.0.proc_symlink())
    }
}
