// SPDX-License-Identifier: MPL-2.0

use ostd::{sync::RcuOption, task::disable_preempt};

use crate::{
    fs::{
        path::{dentry::Dentry, MountLockGuard, Path, MOUNT_LOCK},
        utils::{FileSystem, InodeType},
    },
    prelude::*,
    util::rcu_linked_list::{RcuList, RcuListLink},
};

/// The `MountNode` is used to form a mount tree to maintain the mount information.
pub struct MountNode {
    /// Root dentry.
    root_dentry: Arc<Dentry>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint: RcuOption<Arc<Dentry>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The parent mount node.
    parent: RcuOption<Weak<MountNode>>,
    /// Child mount nodes which are mounted on one dentry of self.
    children: RcuList<MountNode>,
    /// The link used when the is added to the children list.
    children_list_link: RcuListLink<MountNode>,
    /// Reference to self.
    this: Weak<Self>,
}

impl MountNode {
    /// Creates a root mount node with an associated FS.
    ///
    /// The root mount node is not mounted on other mount nodes (which means it has no
    /// parent). The root inode of the fs will form the inner root dentry.
    ///
    /// It is allowed to create a mount node even if the fs has been provided to another
    /// mount node. It is the fs's responsibility to ensure the data consistency.
    pub fn new_root(fs: Arc<dyn FileSystem>) -> Arc<Self> {
        Self::new(fs, None, None)
    }

    pub(super) fn new_child(&self, fs: Arc<dyn FileSystem>) -> Arc<Self> {
        Self::new(fs, None, Some(self.this.clone()))
    }

    /// The internal constructor.
    ///
    /// Root mount node has no mountpoint which other mount nodes must have mountpoint.
    ///
    /// Here, a `MountNode` is instantiated without an initial mountpoint,
    /// avoiding fixed mountpoint limitations. This allows the root mount node to
    /// exist without a mountpoint, ensuring uniformity and security, while all other
    /// mount nodes must be explicitly assigned a mountpoint to maintain structural integrity.
    fn new(
        fs: Arc<dyn FileSystem>,
        root_dentry: Option<Arc<Dentry>>,
        parent_mount: Option<Weak<MountNode>>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: root_dentry.unwrap_or_else(|| Dentry::new_root(fs.root_inode())),
            mountpoint: RcuOption::new_none(),
            fs,
            parent: RcuOption::new(parent_mount),
            children: RcuList::new(|node| &node.children_list_link),
            children_list_link: RcuListLink::default(),
            this: weak_self.clone(),
        })
    }

    /// Mounts a fs on the mountpoint, it will create a new child mount node.
    ///
    /// If the given mountpoint has already been mounted, then its mounted child mount
    /// node will be updated.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    ///
    /// It is allowed to mount a fs even if the fs has been provided to another
    /// mountpoint. It is the fs's responsibility to ensure the data consistency.
    ///
    /// Return the mounted child mount.
    pub(super) fn mount(&self, child_mount: Arc<MountNode>, mount_guard: &MountLockGuard) {
        self.children.push_front(child_mount.clone(), mount_guard);
    }

    /// Unmounts a child mount node from the mountpoint and returns it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub(super) fn unmount(
        &self,
        mountpoint: &Arc<Dentry>,
        mount_guard: &MountLockGuard,
    ) -> Option<Arc<Self>> {
        let target_mount = self.find_child_mount(mountpoint);

        if let Some(mount_node) = &target_mount {
            self.children.remove(mount_node, mount_guard);
        }

        target_mount
    }

    /// Clones a mount tree starting from the specified root `Dentry`.
    ///
    /// The new mount tree will replicate the structure of the original tree.
    /// The new tree is a separate entity rooted at the given `Dentry`,
    /// and the original tree remains unchanged.
    ///
    /// If `recursive` is set to `true`, the entire tree will be copied.
    /// Otherwise, only the root mount node will be copied.
    pub(super) fn clone_mount_node_tree(
        &self,
        root_dentry: &Arc<Dentry>,
        recursive: bool,
    ) -> Arc<Self> {
        let new_root_mount = Self::new(self.fs.clone(), Some(root_dentry.clone()), None);
        if !recursive {
            return new_root_mount;
        }

        let mut clone_pair_list = vec![(self.this(), new_root_mount.clone())];
        while let Some((src_node, dst_node)) = clone_pair_list.pop() {
            let guard = disable_preempt();
            for src_child in src_node.children.iter(&guard) {
                let mountpoint = src_child.mountpoint().unwrap();
                if !mountpoint.is_descendant_of(root_dentry) {
                    continue;
                }

                let dst_child = Self::new(self.fs.clone(), Some(root_dentry.clone()), None);
                dst_child.set_parent(&dst_node);
                dst_node.children.push_front(dst_child.clone(), &guard);
                dst_child.set_mountpoint(mountpoint);
                clone_pair_list.push((src_child, dst_child));
            }
        }

        new_root_mount
    }

    /// Detaches the mount node from the parent mount node.
    fn detach_from_parent(&self, mount_guard: &MountLockGuard) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            parent.children.remove(self, mount_guard);
        }
    }

    /// Attaches the mount node to the mountpoint.
    fn attach_to_mountpoint(&self, mountpoint: &Path, mount_guard: &MountLockGuard) {
        mountpoint
            .mount_node()
            .children
            .push_front(self.this(), mount_guard);
        self.set_parent(mountpoint.mount_node());

        self.set_mountpoint(mountpoint.dentry.clone());
    }

    /// Grafts the mount node tree to the mountpoint.
    pub fn graft_mount_node_tree(&self, mountpoint: &Path) -> Result<()> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let mount_lock = MOUNT_LOCK.lock();
        self.detach_from_parent(&mount_lock);
        self.attach_to_mountpoint(mountpoint, &mount_lock);
        Ok(())
    }

    /// Gets a child mount node from the mountpoint if any.
    pub(super) fn find_child_mount(&self, mountpoint: &Arc<Dentry>) -> Option<Arc<Self>> {
        let guard = disable_preempt();
        self.children.iter(&guard).find(|child| {
            child
                .mountpoint()
                .is_some_and(|child_mountpoint| Arc::ptr_eq(mountpoint, &child_mountpoint))
        })
    }
    /// Gets the root `Dentry` of this mount node.
    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    /// Gets the mountpoint `Dentry` of this mount node if any.
    pub(super) fn mountpoint(&self) -> Option<Arc<Dentry>> {
        self.mountpoint.read().get().as_deref().cloned()
    }

    /// Sets the mountpoint.
    ///
    /// In some cases we may need to reset the mountpoint of
    /// the created `MountNode`, such as move mount.
    pub(super) fn set_mountpoint(&self, dentry: Arc<Dentry>) {
        self.mountpoint.update(Some(dentry.clone()));
        dentry.add_mount();
    }

    /// Flushes all pending filesystem metadata and cached file data to the device.
    pub fn sync(&self) -> Result<()> {
        let guard = disable_preempt();
        let children: Vec<Arc<MountNode>> = self.children.iter(&guard).collect();
        drop(guard);

        for child in children {
            child.sync()?;
        }

        self.fs.sync()?;
        Ok(())
    }

    /// Gets the parent mount node if any.
    pub fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().get().as_deref().cloned()
    }

    /// Sets the parent mount node.
    ///
    /// In some cases we may need to reset the parent of
    /// the created MountNode, such as move mount.
    pub fn set_parent(&self, mount_node: &Arc<MountNode>) {
        self.parent.update(Some(Arc::downgrade(mount_node)));
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Gets the associated fs.
    pub fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }
}

impl Debug for MountNode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("MountNode")
            .field("root", &self.root_dentry)
            .field("fs", &self.fs)
            .finish()
    }
}
