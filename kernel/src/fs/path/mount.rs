// SPDX-License-Identifier: MPL-2.0

use hashbrown::HashMap;

use crate::{
    fs::{
        path::dentry::{Dentry, DentryKey, Dentry_},
        utils::{FileSystem, InodeType},
    },
    prelude::*,
};

/// The `MountNode` is used to form a mount tree to maintain the mount information.
pub struct MountNode {
    /// Root dentry.
    root_dentry: Arc<Dentry_>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint_dentry: RwLock<Option<Arc<Dentry_>>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The parent mount node.
    parent: RwLock<Option<Weak<MountNode>>>,
    /// Child mount nodes which are mounted on one dentry of self.
    children: RwLock<HashMap<DentryKey, Arc<Self>>>,
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
        Self::new(fs, None)
    }

    /// The internal constructor.
    ///
    /// Root mount node has no mountpoint which other mount nodes must have mountpoint.
    ///
    /// Here, a MountNode is instantiated without an initial mountpoint,
    /// avoiding fixed mountpoint limitations. This allows the root mount node to
    /// exist without a mountpoint, ensuring uniformity and security, while all other
    /// mount nodes must be explicitly assigned a mountpoint to maintain structural integrity.
    fn new(fs: Arc<dyn FileSystem>, parent_mount: Option<Weak<MountNode>>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: Dentry_::new_root(fs.root_inode()),
            mountpoint_dentry: RwLock::new(None),
            parent: RwLock::new(parent_mount),
            children: RwLock::new(HashMap::new()),
            fs,
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
    pub fn mount(&self, fs: Arc<dyn FileSystem>, mountpoint: &Dentry) -> Result<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let key = mountpoint.key();
        let child_mount = Self::new(fs, Some(Arc::downgrade(mountpoint.mount_node())));
        self.children.write().insert(key, child_mount.clone());
        Ok(child_mount)
    }

    /// Unmounts a child mount node from the mountpoint and returns it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub fn unmount(&self, mountpoint: &Dentry) -> Result<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }

        let child_mount = self
            .children
            .write()
            .remove(&mountpoint.key())
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;
        Ok(child_mount)
    }

    /// Clones a mount node with the an root `Dentry_`.
    ///
    /// The new mount node will have the same fs as the original one and
    /// have no parent and children. We should set the parent and children manually.
    fn clone_mount_node(&self, root_dentry: &Arc<Dentry_>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: root_dentry.clone(),
            mountpoint_dentry: RwLock::new(None),
            parent: RwLock::new(None),
            children: RwLock::new(HashMap::new()),
            fs: self.fs.clone(),
            this: weak_self.clone(),
        })
    }

    /// Clones a mount tree starting from the specified root `Dentry_`.
    ///
    /// The new mount tree will replicate the structure of the original tree.
    /// The new tree is a separate entity rooted at the given `Dentry_`,
    /// and the original tree remains unchanged.
    ///
    /// If `recursive` is set to `true`, the entire tree will be copied.
    /// Otherwise, only the root mount node will be copied.
    pub(super) fn clone_mount_node_tree(
        &self,
        root_dentry: &Arc<Dentry_>,
        recursive: bool,
    ) -> Arc<Self> {
        let new_root_mount = self.clone_mount_node(root_dentry);
        if !recursive {
            return new_root_mount;
        }

        let mut stack = vec![self.this()];
        let mut new_stack = vec![new_root_mount.clone()];
        while let Some(old_mount) = stack.pop() {
            let new_parent_mount = new_stack.pop().unwrap();
            let old_children = old_mount.children.read();
            for old_child_mount in old_children.values() {
                let mountpoint_dentry = old_child_mount.mountpoint_dentry().unwrap();
                if !mountpoint_dentry.is_descendant_of(old_mount.root_dentry()) {
                    continue;
                }
                let new_child_mount =
                    old_child_mount.clone_mount_node(old_child_mount.root_dentry());
                let key = mountpoint_dentry.key();
                new_parent_mount
                    .children
                    .write()
                    .insert(key, new_child_mount.clone());
                new_child_mount.set_parent(&new_parent_mount);
                new_child_mount
                    .set_mountpoint_dentry(&old_child_mount.mountpoint_dentry().unwrap());
                stack.push(old_child_mount.clone());
                new_stack.push(new_child_mount);
            }
        }

        new_root_mount
    }

    /// Detaches the mount node from the parent mount node.
    fn detach_mount_node(&self) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            parent
                .children
                .write()
                .remove(&self.mountpoint_dentry().unwrap().key());
        }
    }

    /// Attaches the mount node to the mountpoint.
    fn attach_mount_node(&self, mountpoint: &Dentry) {
        let key = mountpoint.key();
        mountpoint
            .mount_node()
            .children
            .write()
            .insert(key, self.this());
        self.set_parent(mountpoint.mount_node());
        mountpoint.set_mountpoint(self.this());
    }

    /// Grafts the mount node tree to the mountpoint.
    pub fn graft_mount_node_tree(&self, mountpoint: &Dentry) -> Result<()> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        self.detach_mount_node();
        self.attach_mount_node(mountpoint);
        Ok(())
    }

    /// Gets a child mount node from the mountpoint if any.
    pub fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return None;
        }
        self.children.read().get(&mountpoint.key()).cloned()
    }

    /// Gets the root `Dentry_` of this mount node.
    pub fn root_dentry(&self) -> &Arc<Dentry_> {
        &self.root_dentry
    }

    /// Gets the mountpoint `Dentry_` of this mount node if any.
    pub fn mountpoint_dentry(&self) -> Option<Arc<Dentry_>> {
        self.mountpoint_dentry.read().clone()
    }

    /// Sets the mountpoint.
    ///
    /// In some cases we may need to reset the mountpoint of
    /// the created `MountNode`, such as move mount.
    pub fn set_mountpoint_dentry(&self, inner: &Arc<Dentry_>) {
        let mut mountpoint_dentry = self.mountpoint_dentry.write();
        *mountpoint_dentry = Some(inner.clone());
    }

    /// Flushes all pending filesystem metadata and cached file data to the device.
    pub fn sync(&self) -> Result<()> {
        let children: Vec<Arc<MountNode>> = {
            let children = self.children.read();
            children.values().cloned().collect()
        };
        for child in children {
            child.sync()?;
        }

        self.fs.sync()?;
        Ok(())
    }

    /// Gets the parent mount node if any.
    pub fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    /// Sets the parent mount node.
    ///
    /// In some cases we may need to reset the parent of
    /// the created MountNode, such as move mount.
    pub fn set_parent(&self, mount_node: &Arc<MountNode>) {
        let mut parent = self.parent.write();
        *parent = Some(Arc::downgrade(mount_node));
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
            .field("mountpoint", &self.mountpoint_dentry)
            .field("fs", &self.fs)
            .finish()
    }
}
