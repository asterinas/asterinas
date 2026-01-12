// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::{
        path::{Mount, Path, PathResolver},
        ramfs::RamFs,
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
}

impl MountNamespace {
    /// Returns a reference to the singleton initial mount namespace.
    #[doc(hidden)]
    pub fn get_init_singleton() -> &'static Arc<MountNamespace> {
        static INIT: Once<Arc<MountNamespace>> = Once::new();

        INIT.call_once(|| {
            let owner = UserNamespace::get_init_singleton().clone();
            let rootfs = RamFs::new();

            Arc::new_cyclic(|weak_self| {
                let root = Mount::new_root(rootfs, weak_self.clone());
                MountNamespace { root, owner }
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
            let new_root =
                root_mount.clone_mount_tree(root_mount.root_dentry(), Some(weak_self), true);

            MountNamespace {
                root: new_root,
                owner,
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

    /// Returns the owner user namespace of the namespace.
    pub fn owner(&self) -> &Arc<UserNamespace> {
        &self.owner
    }

    /// Checks whether a given mount belongs to this mount namespace.
    pub fn owns(self: &Arc<Self>, mount: &Mount) -> bool {
        mount.mnt_ns().as_ptr() == Arc::as_ptr(self)
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
