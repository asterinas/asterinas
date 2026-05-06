// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use crate::{
    fs::{
        cgroupfs::CgroupNamespace,
        file::{InodeType, mkmod},
        procfs::{
            pid::TidDirOps,
            template::{
                DirOps, ListedEntry, ProcDir, ProcSym, ReaddirEntry, SymOps, visit_listed_entries,
            },
        },
        pseudofs::NsCommonOps,
        vfs::{
            inode::{Inode, RevalidationPolicy, SymbolicLink},
            path::{MountNamespace, Path},
        },
    },
    ipc::IpcNamespace,
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{NsProxy, UserNamespace, posix_thread::AsPosixThread},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/ns` (and also `/proc/[pid]/ns`).
pub(super) struct NsDirOps {
    dir: TidDirOps,
}

impl NsDirOps {
    /// Creates a new directory inode for the `ns` directory.
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(
            Self { dir: dir.clone() },
            parent,
            // Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/proc/base.c#L3321>
            mkmod!(u+r, a+x),
        )
    }
}

/// Namespace entries backed by the thread's [`NsProxy`].
#[derive(Clone, Copy)]
enum NsProxyEntry {
    /// The cgroup namespace.
    Cgroup,
    /// The IPC namespace.
    Ipc,
    /// The mount namespace.
    Mnt,
    /// The UTS namespace.
    Uts,
}

impl NsProxyEntry {
    /// All supported `NsProxy`-backed namespace entries.
    const ALL: &[Self] = &[Self::Cgroup, Self::Ipc, Self::Mnt, Self::Uts];

    /// Returns the filename of this namespace entry under `/proc/[pid]/ns/`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Cgroup => "cgroup",
            Self::Ipc => "ipc",
            Self::Mnt => "mnt",
            Self::Uts => "uts",
        }
    }

    /// Parses a namespace entry name, returning `None` for unrecognized names.
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "cgroup" => Some(Self::Cgroup),
            "ipc" => Some(Self::Ipc),
            "mnt" => Some(Self::Mnt),
            "uts" => Some(Self::Uts),
            _ => None,
        }
    }

    /// Creates a symlink inode for this namespace entry.
    fn new_sym_inode(self, ns_proxy: &NsProxy, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        match self {
            Self::Cgroup => {
                NsSymOps::<CgroupNamespace>::new_inode(ns_proxy.cgroup_ns().get_path(), parent)
            }
            Self::Ipc => NsSymOps::<IpcNamespace>::new_inode(ns_proxy.ipc_ns().get_path(), parent),
            Self::Mnt => {
                NsSymOps::<MountNamespace>::new_inode(ns_proxy.mnt_ns().get_path(), parent)
            }
            Self::Uts => NsSymOps::<UtsNamespace>::new_inode(ns_proxy.uts_ns().get_path(), parent),
        }
    }
}

/// Extracts the cached namespace path from a `NsSymlink<T>` inode.
///
/// Returns `None` if the inode is not a known namespace symlink type.
fn cached_ns_path(inode: &dyn Inode) -> Option<&Path> {
    if let Some(sym) = inode.downcast_ref::<NsSymlink<CgroupNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<IpcNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<MountNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UserNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UtsNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    // TODO: Support additional namespace types.
    None
}

impl DirOps for NsDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if name == "user" {
            let Some(process) = self.dir.process() else {
                return_errno_with_message!(Errno::ESRCH, "the process does not exist");
            };

            let user_ns = process.user_ns().lock();
            return Ok(NsSymOps::<UserNamespace>::new_inode(
                user_ns.get_path(),
                this_dir.this_weak().clone(),
            ));
        }

        // Validate the name and get the current namespace path.
        let entry = NsProxyEntry::from_str(name)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the file does not exist"))?;

        let Some(thread) = self.dir.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };

        let ns_proxy_guard = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let ns_proxy = ns_proxy_guard
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the thread has exited"))?;
        Ok(entry.new_sym_inode(ns_proxy, this_dir.this_weak().clone()))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        let Some(thread) = self.dir.thread() else {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        };

        let has_ns_proxy = thread
            .as_posix_thread()
            .unwrap()
            .ns_proxy()
            .lock()
            .as_ref()
            .is_some();
        let ns_proxy_entries = has_ns_proxy
            .then_some(NsProxyEntry::ALL)
            .into_iter()
            .flat_map(|entries| {
                entries
                    .iter()
                    .map(|entry| ListedEntry::new(entry.as_str(), InodeType::SymLink))
            });

        let user_entry = Some(ListedEntry::new("user", InodeType::SymLink)).into_iter();

        visit_listed_entries(offset, ns_proxy_entries.chain(user_entry), visit_fn)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        // Files in the `ns` directory will not appear implicitly. They can only disappear
        // implicitly. Therefore, it is sufficient to revalidate only their existence.
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, child: &dyn Inode) -> bool {
        let Some(cached_path) = cached_ns_path(child) else {
            return false;
        };

        if child.downcast_ref::<NsSymlink<UserNamespace>>().is_some() {
            let Some(process) = self.dir.process() else {
                return false;
            };
            let user_ns = process.user_ns().lock();
            return cached_path == &user_ns.get_path();
        }

        let Some(thread) = self.dir.thread() else {
            return false;
        };
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let Some(ns_proxy) = ns_proxy.as_ref() else {
            return false;
        };

        if child.downcast_ref::<NsSymlink<CgroupNamespace>>().is_some() {
            return cached_path == &ns_proxy.cgroup_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<MountNamespace>>().is_some() {
            return cached_path == &ns_proxy.mnt_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<UtsNamespace>>().is_some() {
            return cached_path == &ns_proxy.uts_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<IpcNamespace>>().is_some() {
            return cached_path == &ns_proxy.ipc_ns().get_path();
        }

        // TODO: Support additional namespace types.
        false
    }
}

type NsSymlink<T> = ProcSym<NsSymOps<T>>;

/// Represents the inode at `/proc/[pid]/task/[tid]/ns/<type>` (and also `/proc/[pid]/ns/<type>`).
struct NsSymOps<T: NsCommonOps> {
    ns_path: Path,
    phantom: PhantomData<T>,
}

impl<T: NsCommonOps> NsSymOps<T> {
    /// Creates a new symlink inode pointing to the given namespace.
    fn new_inode(ns_path: Path, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(
            Self {
                ns_path,
                phantom: PhantomData,
            },
            parent,
            // Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/proc/namespaces.c#L105>
            mkmod!(a+rwx),
        )
    }
}

impl<T: NsCommonOps> SymOps for NsSymOps<T> {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Path(self.ns_path.clone()))
    }
}
