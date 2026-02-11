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

impl NsDirOps {
    /// Namespace entries backed by the thread's [`NsProxy`].
    #[expect(clippy::type_complexity)]
    const NS_PROXY_ENTRIES: &[(&str, fn(&NsProxy, Weak<dyn Inode>) -> Arc<dyn Inode>)] = &[
        ("uts", |proxy, parent| {
            NsSymOps::new_inode(proxy.uts_ns(), parent)
        }),
        ("mnt", |proxy, parent| {
            NsSymOps::new_inode(proxy.mnt_ns(), parent)
        }),
    ];

    /// Looks up a namespace symlink backed by the thread's [`NsProxy`].
    fn lookup_ns_proxy_child(&self, name: &str, parent: Weak<dyn Inode>) -> Result<Arc<dyn Inode>> {
        let constructor = Self::NS_PROXY_ENTRIES
            .iter()
            .find(|(entry_name, _)| *entry_name == name)
            .map(|(_, ctor)| ctor)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the file does not exist"))?;

        let thread = self.dir.thread();
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let ns_proxy = ns_proxy.as_ref().ok_or_else(|| {
            Error::with_message(
                Errno::ENOENT,
                "the thread's namespace proxy no longer exists",
            )
        })?;

        Ok(constructor(ns_proxy, parent))
    }

    /// Creates a symlink inode for the process's user namespace.
    fn new_user_ns_inode(&self, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let user_ns = self.dir.process_ref.user_ns().lock();
        NsSymOps::new_inode(&*user_ns, parent)
    }
}

impl DirOps for NsDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = if name == "user" {
            self.new_user_ns_inode(dir.this_weak().clone())
        } else {
            self.lookup_ns_proxy_child(name, dir.this_weak().clone())?
        };

        let mut cached_children = dir.cached_children().write();
        cached_children.remove_entry_by_name(name);
        cached_children.put((name.to_string(), inode.clone()));
        Ok(inode)
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();

        // Refresh NsProxy-backed entries: remove stale ones and re-add
        // if the proxy is still alive.
        let thread = self.dir.thread();
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();

        for &(name, constructor) in Self::NS_PROXY_ENTRIES {
            cached_children.remove_entry_by_name(name);
            if let Some(ns_proxy) = ns_proxy.as_ref() {
                let inode = constructor(ns_proxy, dir.this_weak().clone());
                cached_children.put((name.to_string(), inode));
            }
        }

        drop(ns_proxy);

        // Refresh the user namespace entry unconditionally.
        cached_children.remove_entry_by_name("user");
        let user_inode = self.new_user_ns_inode(dir.this_weak().clone());
        cached_children.put(("user".to_string(), user_inode));

        cached_children.downgrade()
    }

    fn validate_child(&self, child: &dyn Inode) -> bool {
        if let Some(sym) = child.downcast_ref::<NsSymlink<UserNamespace>>() {
            let user_ns = self.dir.process_ref.user_ns().lock();
            return &sym.inner().ns_path == user_ns.path();
        }

        let thread = self.dir.thread();
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let Some(ns_proxy) = ns_proxy.as_ref() else {
            return false;
        };

        if let Some(sym) = child.downcast_ref::<NsSymlink<UtsNamespace>>() {
            return &sym.inner().ns_path == ns_proxy.uts_ns().path();
        }

        if let Some(sym) = child.downcast_ref::<NsSymlink<MountNamespace>>() {
            return &sym.inner().ns_path == ns_proxy.mnt_ns().path();
        }

        // TODO: Support additional namespace types.
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
    fn new_inode(ns: &Arc<T>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSymBuilder::new(
            Self {
                ns_path: ns.path().clone(),
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
