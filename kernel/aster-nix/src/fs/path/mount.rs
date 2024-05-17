// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        path::dentry::{Dentry, DentryKey, Dentry_},
        utils::{FileSystem, InodeType},
    },
    prelude::*,
};

/// The MountNode can form a mount tree to maintain the mount information.
pub struct MountNode {
    /// Root Dentry_.
    root_dentry: Arc<Dentry_>,
    /// Mountpoint Dentry_. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint_dentry: RwLock<Option<Arc<Dentry_>>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The parent mount node.
    parent: RwLock<Option<Weak<MountNode>>>,
    /// Child mount nodes which are mounted on one dentry of self.
    children: Mutex<BTreeMap<DentryKey, Arc<Self>>>,
    /// Reference to self.
    this: Weak<Self>,
}

impl MountNode {
    /// Create a root mount node with an associated FS.
    ///
    /// The root mount node is not mounted on other mount nodes(which means it has no
    /// parent). The root inode of the fs will form the root dentryinner of it.
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
            children: Mutex::new(BTreeMap::new()),
            fs,
            this: weak_self.clone(),
        })
    }

    /// Mount an fs on the mountpoint, it will create a new child mount node.
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
    pub fn mount(&self, fs: Arc<dyn FileSystem>, mountpoint: &Arc<Dentry>) -> Result<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let key = mountpoint.key();
        let child_mount = Self::new(fs, Some(Arc::downgrade(mountpoint.mount_node())));
        self.children.lock().insert(key, child_mount.clone());
        Ok(child_mount)
    }

    /// Unmount a child mount node from the mountpoint and return it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub fn umount(&self, mountpoint: &Dentry) -> Result<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }

        let child_mount = self
            .children
            .lock()
            .remove(&mountpoint.key())
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;
        Ok(child_mount)
    }

    /// Clone a mount node with the an root Dentry_.
    /// The new mount node will have the same fs as the original one.
    /// The new mount node root Dentry_ will be the given one.
    /// The new mount node will have no parent and children.
    /// We should set the parent and children manually.
    pub fn clone_mount(&self, root_dentry: Arc<Dentry_>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry,
            mountpoint_dentry: RwLock::new(None),
            parent: RwLock::new(None),
            children: Mutex::new(BTreeMap::new()),
            fs: self.fs.clone(),
            this: weak_self.clone(),
        })
    }

    /// Copy a mount tree with the given root Dentry_.
    /// The new mount tree will have the same structure as the original one.
    /// But the new mount tree is a subtree which is rooted at the given root Dentry_.
    /// The new mount tree is a new tree, which means the original one is not affected.
    pub fn copy_mount_tree(&self, root_dentry: Arc<Dentry_>) -> Arc<Self> {
        let new_root_mount = self.clone_mount(root_dentry);
        let mut stack = vec![self.this()];
        let mut new_stack = vec![new_root_mount.clone()];

        let mut dentry = new_root_mount.root_dentry.clone();
        while let Some(old_mount) = stack.pop() {
            let new_parent_mount = new_stack.pop().unwrap().clone();
            let old_children = old_mount.children.lock();
            for old_child_mount in old_children.values() {
                let mountpoint_dentry = old_child_mount.mountpoint_dentry().unwrap();
                if !dentry.is_subdir(mountpoint_dentry.clone()) {
                    continue;
                }
                stack.push(old_child_mount.clone());
                let new_child_mount =
                    old_child_mount.clone_mount(old_child_mount.root_dentry.clone());
                let key = mountpoint_dentry.key();
                new_parent_mount
                    .children
                    .lock()
                    .insert(key, new_child_mount.clone());
                new_child_mount.set_parent(new_parent_mount.clone());
                new_child_mount
                    .set_mountpoint_dentry(old_child_mount.mountpoint_dentry().unwrap().clone());
                new_stack.push(new_child_mount.clone());
                dentry = new_child_mount.root_dentry.clone();
            }
        }
        new_root_mount.clone()
    }

    /// Unattach the mount node from the parent mount node.
    fn unattach_mount(&self) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            parent
                .children
                .lock()
                .remove(&self.mountpoint_dentry().unwrap().key());
        }
    }

    /// Attach the mount node to the mountpoint.
    fn attach_mount(&self, mountpoint: Arc<Dentry>) {
        let key = mountpoint.key();
        mountpoint
            .mount_node()
            .children
            .lock()
            .insert(key, self.this());
        self.set_parent(mountpoint.mount_node().clone());
        mountpoint.set_mountpoint(self.this());
    }

    /// Graft the mount tree to the mountpoint.
    pub fn graft_mount_tree(&self, mountpoint: Arc<Dentry>) -> Result<()> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        self.unattach_mount();
        self.attach_mount(mountpoint.clone());
        Ok(())
    }

    /// Try to get a child mount node from the mountpoint.
    pub fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        if !Arc::ptr_eq(mountpoint.mount_node(), &self.this()) {
            return None;
        }
        self.children.lock().get(&mountpoint.key()).cloned()
    }

    /// Get the root Dentry_ of this mount node.
    pub fn root_dentry(&self) -> &Arc<Dentry_> {
        &self.root_dentry
    }

    /// Try to get the mountpoint Dentry_ of this mount node.
    pub fn mountpoint_dentry(&self) -> Option<Arc<Dentry_>> {
        self.mountpoint_dentry.read().clone()
    }

    /// Set the mountpoint.
    ///
    /// In some cases we may need to reset the mountpoint of
    /// the created MountNode, such as move mount.
    pub fn set_mountpoint_dentry(&self, inner: Arc<Dentry_>) {
        let mut mountpoint_dentry = self.mountpoint_dentry.write();
        *mountpoint_dentry = Some(inner);
    }

    /// Flushes all pending filesystem metadata and cached file data to the device.
    pub fn sync(&self) -> Result<()> {
        let children = self.children.lock();
        for child in children.values() {
            child.sync()?;
        }
        drop(children);

        self.fs.sync()?;
        Ok(())
    }

    /// Try to get the parent mount node.
    pub fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    /// Set the parent.
    ///
    /// In some cases we may need to reset the parent of
    /// the created MountNode, such as move mount.
    pub fn set_parent(&self, mount_node: Arc<MountNode>) {
        let mut parent = self.parent.write();
        *parent = Some(Arc::downgrade(&mount_node));
    }

    /// Get strong reference to self.
    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Get the associated fs.
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
