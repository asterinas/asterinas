// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use super::{mount::MountNsFileCopying, try_get_mnt_ns_inode};
use crate::{
    fs::{
        fs_impls::ramfs::RamFs,
        pseudofs::{NsCommonOps, NsType, StashedDentry},
        vfs::path::{Dentry, Mount, Path, PathResolver},
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// Represents a mount namespace, which encapsulates a mount tree and provides
/// isolation for filesystem views between different threads.
///
/// A `MountNamespace` only allows operations on [`Mount`]s that belong to that `MountNamespace`.
/// If the operation target includes [`Mount`]s from other `MountNamespace`s, it will be directly
/// rejected and return an `Err`.
pub struct MountNamespace {
    /// The root mount of this namespace.
    root: Arc<Mount>,
    /// The user namespace that owns this mount namespace.
    owner: Arc<UserNamespace>,
    /// The stashed dentry in nsfs.
    stashed_dentry: StashedDentry,
}

impl PartialEq for MountNamespace {
    fn eq(&self, other: &Self) -> bool {
        self.stashed_dentry == other.stashed_dentry
    }
}

impl Eq for MountNamespace {}

impl PartialOrd for MountNamespace {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MountNamespace {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.stashed_dentry.cmp(&other.stashed_dentry)
    }
}

impl MountNamespace {
    /// Returns a reference to the singleton initial mount namespace.
    #[doc(hidden)]
    pub fn get_init_singleton() -> &'static Arc<MountNamespace> {
        static INIT: Once<Arc<MountNamespace>> = Once::new();

        INIT.call_once(|| {
            let owner = UserNamespace::get_init_singleton().clone();
            let rootfs = RamFs::new_rootfs();

            Arc::new_cyclic(|weak_self| {
                let root = Mount::new_root(rootfs, weak_self.clone());
                let stashed_dentry = StashedDentry::new();
                MountNamespace {
                    root,
                    owner,
                    stashed_dentry,
                }
            })
        })
    }

    /// Gets the root mount of this namespace.
    pub fn root(&self) -> &Arc<Mount> {
        &self.root
    }

    /// Creates a new filesystem resolver for this namespace.
    ///
    /// The resolver is initialized with the root and current working directory
    /// both set to the **effective root** of this mount namespace.
    ///
    /// The "effective root" refers to the currently visible root directory, which
    /// may differ from the original root filesystem if overlay mounts exist.
    pub fn new_path_resolver(&self) -> PathResolver {
        let root = Path::new_fs_root(self.root.clone()).get_top_path();
        let cwd = Path::new_fs_root(self.root.clone()).get_top_path();
        PathResolver::new(root, cwd)
    }

    /// Creates a deep copy of this mount namespace, including the entire mount tree.
    ///
    /// This is typically used when creating a new namespace for a process or thread.
    pub fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<MountNamespace>> {
        owner.check_cap(CapSet::SYS_ADMIN, posix_thread)?;

        let root_mount = &self.root;
        let new_mnt_ns = Arc::new_cyclic(|weak_self| {
            let new_root = root_mount.clone_mount_tree(
                root_mount.root_dentry(),
                weak_self,
                true,
                MountNsFileCopying::Skip,
            );
            let stashed_dentry = StashedDentry::new();
            MountNamespace {
                root: new_root,
                owner,
                stashed_dentry,
            }
        });

        Ok(new_mnt_ns)
    }

    /// Flushes all pending filesystem metadata and cached file data to the device
    /// for all mounted filesystems in this mount namespace.
    pub fn sync(&self) -> Result<()> {
        let mut mount_queue = VecDeque::new();
        let mut visited_filesystems = hashbrown::HashSet::new();
        mount_queue.push_back(self.root.clone());

        while let Some(current_mount) = mount_queue.pop_front() {
            let fs_ptr = Arc::as_ptr(current_mount.fs());
            // Only sync each filesystem once.
            if visited_filesystems.insert(fs_ptr) {
                current_mount.sync()?;
            }

            let children = current_mount.children.read();
            for child_mount in children.values() {
                mount_queue.push_back(child_mount.clone());
            }
        }

        Ok(())
    }

    /// Checks whether a given mount belongs to this mount namespace.
    pub fn owns(self: &Arc<Self>, mount: &Mount) -> bool {
        mount.mnt_ns().as_ptr() == Arc::as_ptr(self)
    }

    /// Returns whether bind-mounting or moving `dentry` into this mount namespace
    /// would create a mount-namespace loop.
    pub(super) fn would_form_mnt_ns_loop(&self, dentry: &Dentry) -> bool {
        let Some(mnt_ns_inode) = try_get_mnt_ns_inode(dentry) else {
            return false;
        };

        self >= mnt_ns_inode.ns().as_ref()
    }

    /// Ensures that importing the mount subtree rooted at `root_mount` into this
    /// mount namespace would not form a mount-namespace loop.
    pub(super) fn check_no_mnt_ns_loop_in_tree(&self, root_mount: &Arc<Mount>) -> Result<()> {
        let mut worklist = VecDeque::new();
        worklist.push_back(root_mount.clone());

        let mut checked_root_dentries = BTreeSet::new();

        while let Some(mount) = worklist.pop_front() {
            let root_dentry = mount.root_dentry();
            if checked_root_dentries.insert(root_dentry.key())
                && self.would_form_mnt_ns_loop(root_dentry)
            {
                return_errno_with_message!(
                    Errno::ELOOP,
                    "the mount tree contains a mount namespace file that would create a namespace loop"
                );
            }
            let children = mount.children.read();
            worklist.extend(children.values().cloned());
        }

        Ok(())
    }
}

// When a mount namespace is dropped, it means that the corresponding mount
// tree is no longer valid. Therefore, all mounts in its mount tree should be
// detached from their parents and cleared of their mountpoints.
impl Drop for MountNamespace {
    fn drop(&mut self) {
        let mut worklist = VecDeque::new();
        worklist.push_back(self.root.clone());
        while let Some(current_mount) = worklist.pop_front() {
            let mut children = current_mount.children.write();
            for (_, child) in children.drain() {
                child.set_parent(None);
                child.clear_mountpoint();
                worklist.push_back(child);
            }
        }
    }
}

impl NsCommonOps for MountNamespace {
    const TYPE: NsType = NsType::Mnt;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "a mount namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
