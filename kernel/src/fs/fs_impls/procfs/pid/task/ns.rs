// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;

use crate::{
    fs::{
        cgroupfs::CgroupNamespace,
        file::mkmod,
        procfs::{
            pid::TidDirOps,
            template::{DirOps, ProcDir, ProcDirBuilder, ProcSym, ProcSymBuilder, SymOps},
        },
        pseudofs::NsCommonOps,
        utils::DirEntryVecExt,
        vfs::{
            inode::{Inode, SymbolicLink},
            path::{MountNamespace, Path},
        },
    },
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
        ProcDirBuilder::new(
            Self { dir: dir.clone() },
            // Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/proc/base.c#L3321>
            mkmod!(u+r, a+x),
        )
        .parent(parent)
        .build()
        .unwrap()
    }
}

/// Namespace entries backed by the thread's [`NsProxy`].
#[derive(Clone, Copy)]
enum NsProxyEntry {
    /// The cgroup namespace.
    Cgroup,
    /// The mount namespace.
    Mnt,
    /// The UTS namespace.
    Uts,
}

impl NsProxyEntry {
    /// All supported `NsProxy`-backed namespace entries.
    const ALL: &[Self] = &[Self::Cgroup, Self::Mnt, Self::Uts];

    /// Returns the filename of this namespace entry under `/proc/[pid]/ns/`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Cgroup => "cgroup",
            Self::Mnt => "mnt",
            Self::Uts => "uts",
        }
    }

    /// Parses a namespace entry name, returning `None` for unrecognized names.
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "cgroup" => Some(Self::Cgroup),
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
            Self::Mnt => {
                NsSymOps::<MountNamespace>::new_inode(ns_proxy.mnt_ns().get_path(), parent)
            }
            Self::Uts => NsSymOps::<UtsNamespace>::new_inode(ns_proxy.uts_ns().get_path(), parent),
        }
    }

    /// Returns the current namespace path for this entry.
    fn current_path(self, ns_proxy: &NsProxy) -> Path {
        match self {
            Self::Cgroup => ns_proxy.cgroup_ns().get_path(),
            Self::Mnt => ns_proxy.mnt_ns().get_path(),
            Self::Uts => ns_proxy.uts_ns().get_path(),
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
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let mut cached_children = dir.cached_children().write();

        if name == "user" {
            let current_path = {
                let user_ns = self.dir.process_ref.user_ns().lock();
                user_ns.get_path()
            };
            // Reuse the cached inode if the user namespace hasn't changed.
            if let Some(cached) = cached_children.find_entry_by_name(name)
                && cached_ns_path(&**cached) == Some(&current_path)
            {
                return Ok(cached.clone());
            }

            let inode = NsSymOps::<UserNamespace>::new_inode(current_path, dir.this_weak().clone());
            cached_children.remove_entry_by_name(name);
            cached_children.put((name.to_string(), inode.clone()));
            return Ok(inode);
        }

        // Validate the name and get the current namespace path.
        let entry = NsProxyEntry::from_str(name)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the file does not exist"))?;

        let thread = self.dir.thread();
        let ns_proxy_guard = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let ns_proxy = ns_proxy_guard
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the thread has exited"))?;
        let current_path = entry.current_path(ns_proxy);

        // Reuse the cached inode if the namespace hasn't changed.
        if let Some(cached) = cached_children.find_entry_by_name(name)
            && cached_ns_path(&**cached) == Some(&current_path)
        {
            return Ok(cached.clone());
        }

        let inode = entry.new_sym_inode(ns_proxy, dir.this_weak().clone());
        cached_children.remove_entry_by_name(name);
        cached_children.put((name.to_string(), inode.clone()));
        Ok(inode)
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();

        // Refresh `NsProxy`-backed entries only when the namespace has changed
        // or the proxy has been dropped.
        let thread = self.dir.thread();
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();

        for entry in NsProxyEntry::ALL {
            let name = entry.as_str();
            match ns_proxy.as_ref() {
                Some(ns_proxy) => {
                    let current_path = entry.current_path(ns_proxy);
                    let needs_update = cached_children
                        .find_entry_by_name(name)
                        .is_none_or(|cached| cached_ns_path(&**cached) != Some(&current_path));
                    if needs_update {
                        cached_children.remove_entry_by_name(name);
                        let inode = entry.new_sym_inode(ns_proxy, dir.this_weak().clone());
                        cached_children.put((name.to_string(), inode));
                    }
                }
                None => {
                    // `NsProxy` is gone; remove the stale entry if present.
                    cached_children.remove_entry_by_name(name);
                }
            }
        }

        drop(ns_proxy);

        // Refresh the user namespace entry only when it has changed.
        let user_ns_path = {
            let user_ns = self.dir.process_ref.user_ns().lock();
            user_ns.get_path()
        };
        let user_needs_update = cached_children
            .find_entry_by_name("user")
            .is_none_or(|cached| cached_ns_path(&**cached) != Some(&user_ns_path));
        if user_needs_update {
            cached_children.remove_entry_by_name("user");
            let user_inode =
                NsSymOps::<UserNamespace>::new_inode(user_ns_path, dir.this_weak().clone());
            cached_children.put(("user".to_string(), user_inode));
        }

        cached_children.downgrade()
    }

    fn validate_child(&self, child: &dyn Inode) -> bool {
        let Some(cached_path) = cached_ns_path(child) else {
            return false;
        };

        if child.downcast_ref::<NsSymlink<UserNamespace>>().is_some() {
            let user_ns = self.dir.process_ref.user_ns().lock();
            return cached_path == &user_ns.get_path();
        }

        let thread = self.dir.thread();
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
        ProcSymBuilder::new(
            Self {
                ns_path,
                phantom: PhantomData,
            },
            // Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/proc/namespaces.c#L105>
            mkmod!(a+rwx),
        )
        .parent(parent)
        .build()
        .unwrap()
    }
}

impl<T: NsCommonOps> SymOps for NsSymOps<T> {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Path(self.ns_path.clone()))
    }
}
