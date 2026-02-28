// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;

use crate::{
    fs::{
        path::{MountNamespace, Path},
        procfs::{
            DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps, pid::TidDirOps,
            template::ProcSym,
        },
        pseudofs::NsCommonOps,
        utils::{DirEntryVecExt, Inode, SymbolicLink, mkmod},
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
const NS_PROXY_ENTRIES: &[&str] = &["ipc", "mnt", "uts"];

/// Creates a symlink inode for a namespace entry backed by the thread's [`NsProxy`].
fn new_ns_proxy_sym_inode(
    ns_proxy: &NsProxy,
    name: &str,
    parent: Weak<dyn Inode>,
) -> Arc<dyn Inode> {
    match name {
        "ipc" => NsSymOps::<IpcNamespace>::new_inode(ns_proxy.ipc_ns().get_path(), parent),
        "mnt" => NsSymOps::<MountNamespace>::new_inode(ns_proxy.mnt_ns().get_path(), parent),
        "uts" => NsSymOps::<UtsNamespace>::new_inode(ns_proxy.uts_ns().get_path(), parent),
        _ => unreachable!(),
    }
}

/// Returns the current namespace path for a given `NsProxy` entry name.
fn current_ns_proxy_path(ns_proxy: &NsProxy, name: &str) -> Path {
    match name {
        "ipc" => ns_proxy.ipc_ns().get_path(),
        "mnt" => ns_proxy.mnt_ns().get_path(),
        "uts" => ns_proxy.uts_ns().get_path(),
        _ => unreachable!(),
    }
}

/// Extracts the cached namespace path from a `NsSymlink<T>` inode.
///
/// Returns `None` if the inode is not a known namespace symlink type.
fn cached_ns_path(inode: &dyn Inode) -> Option<&Path> {
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UserNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<IpcNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UtsNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<MountNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
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
            Ok(inode)
        } else {
            // Validate the name and get the current namespace path.
            if !NS_PROXY_ENTRIES.contains(&name) {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the file does not exist",
                ));
            }
            let thread = self.dir.thread();
            let ns_proxy_guard = thread.as_posix_thread().unwrap().ns_proxy().lock();
            let ns_proxy = ns_proxy_guard.as_ref().ok_or_else(|| {
                Error::with_message(
                    Errno::ENOENT,
                    "the thread's namespace proxy no longer exists",
                )
            })?;
            let current_path = current_ns_proxy_path(ns_proxy, name);

            // Reuse the cached inode if the namespace hasn't changed.
            if let Some(cached) = cached_children.find_entry_by_name(name)
                && cached_ns_path(&**cached) == Some(&current_path)
            {
                return Ok(cached.clone());
            }

            let inode = new_ns_proxy_sym_inode(ns_proxy, name, dir.this_weak().clone());
            cached_children.remove_entry_by_name(name);
            cached_children.put((name.to_string(), inode.clone()));
            Ok(inode)
        }
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

        for name in NS_PROXY_ENTRIES {
            match ns_proxy.as_ref() {
                Some(ns_proxy) => {
                    let current_path = current_ns_proxy_path(ns_proxy, name);
                    let needs_update = cached_children
                        .find_entry_by_name(name)
                        .is_none_or(|cached| cached_ns_path(&**cached) != Some(&current_path));
                    if needs_update {
                        cached_children.remove_entry_by_name(name);
                        let inode = new_ns_proxy_sym_inode(ns_proxy, name, dir.this_weak().clone());
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

        if child.downcast_ref::<NsSymlink<UtsNamespace>>().is_some() {
            return cached_path == &ns_proxy.uts_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<MountNamespace>>().is_some() {
            return cached_path == &ns_proxy.mnt_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<IpcNamespace>>().is_some() {
            return cached_path == &ns_proxy.ipc_ns().get_path();
        }

        false
    }
}

type NsSymlink<T> = ProcSym<NsSymOps<T>>;

/// Represents the inode at `/proc/[pid]/task/[tid]/ns/<type>` (and also `/proc/[pid]/ns/<type>`).
pub struct NsSymOps<T: NsCommonOps> {
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
