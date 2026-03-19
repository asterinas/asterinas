// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use crate::{
    fs::{
        file::mkmod,
        procfs::{
            pid::TidDirOps,
            template::{DirOps, ProcDir, ProcDirBuilder, ProcSym, ProcSymBuilder, SymOps},
        },
        pseudofs::NsCommonOps,
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
    /// The UTS namespace.
    Uts,
    /// The mount namespace.
    Mnt,
}

impl NsProxyEntry {
    /// All supported `NsProxy`-backed namespace entries.
    const ALL: &[Self] = &[Self::Uts, Self::Mnt];

    /// Returns the filename of this namespace entry under `/proc/[pid]/ns/`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Uts => "uts",
            Self::Mnt => "mnt",
        }
    }

    /// Parses a namespace entry name, returning `None` for unrecognized names.
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "uts" => Some(Self::Uts),
            "mnt" => Some(Self::Mnt),
            _ => None,
        }
    }

    /// Creates a symlink inode for this namespace entry.
    fn new_sym_inode(self, ns_proxy: &NsProxy, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        match self {
            Self::Uts => NsSymOps::<UtsNamespace>::new_inode(ns_proxy.uts_ns().get_path(), parent),
            Self::Mnt => {
                NsSymOps::<MountNamespace>::new_inode(ns_proxy.mnt_ns().get_path(), parent)
            }
        }
    }
}

/// Extracts the cached namespace path from a `NsSymlink<T>` inode.
///
/// Returns `None` if the inode is not a known namespace symlink type.
fn cached_ns_path(inode: &dyn Inode) -> Option<&Path> {
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UserNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UtsNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<MountNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    // TODO: Support additional namespace types.
    None
}

impl DirOps for NsDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if name == "user" {
            let user_ns = self.dir.process_ref.user_ns().lock();
            return Ok(NsSymOps::<UserNamespace>::new_inode(
                user_ns.get_path(),
                dir.this_weak().clone(),
            ));
        }

        // Validate the name and get the current namespace path.
        let entry = NsProxyEntry::from_str(name)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the file does not exist"))?;

        let thread = self.dir.thread();
        let ns_proxy_guard = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let ns_proxy = ns_proxy_guard
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the thread has exited"))?;
        Ok(entry.new_sym_inode(ns_proxy, dir.this_weak().clone()))
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        let mut children = Vec::new();

        let thread = self.dir.thread();
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();

        if let Some(ns_proxy) = ns_proxy.as_ref() {
            for entry in NsProxyEntry::ALL {
                children.push((
                    entry.as_str().to_string(),
                    entry.new_sym_inode(ns_proxy, dir.this_weak().clone()),
                ));
            }
        }

        let user_ns = self.dir.process_ref.user_ns().lock();
        children.push((
            String::from("user"),
            NsSymOps::<UserNamespace>::new_inode(user_ns.get_path(), dir.this_weak().clone()),
        ));

        children
    }

    fn revalidate_pos_child(&self, _name: &str, child: &dyn Inode) -> bool {
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

        // TODO: Support additional namespace types.
        false
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        if name == "user" {
            return false;
        }

        let Some(_entry) = NsProxyEntry::from_str(name) else {
            return true;
        };

        let thread = self.dir.thread();
        thread
            .as_posix_thread()
            .unwrap()
            .ns_proxy()
            .lock()
            .as_ref()
            .is_none()
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
        .need_revalidation()
        .build()
        .unwrap()
    }
}

impl<T: NsCommonOps> SymOps for NsSymOps<T> {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Path(self.ns_path.clone()))
    }
}
