// SPDX-License-Identifier: MPL-2.0

use hashbrown::HashMap;

use crate::{
    fs::{
        path::{
            dentry::{Dentry, DentryKey},
            Path,
        },
        utils::{FileSystem, InodeType},
    },
    prelude::*,
};

/// A `Mount` represents a mounted filesystem instance in the VFS.
///
/// Each `Mount` can be viewed as a node in the mount tree, maintaining
/// mount-related information and the structure of the mount tree.
pub struct Mount {
    /// Root dentry.
    root_dentry: Arc<Dentry>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint: RwLock<Option<Arc<Dentry>>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The parent mount node.
    parent: RwLock<Option<Weak<Mount>>>,
    /// Child mount nodes which are mounted on one dentry of self.
    children: RwLock<HashMap<DentryKey, Arc<Self>>>,
    /// Reference to self.
    this: Weak<Self>,
}

impl Mount {
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
    /// Here, a Mount is instantiated without an initial mountpoint,
    /// avoiding fixed mountpoint limitations. This allows the root mount node to
    /// exist without a mountpoint, ensuring uniformity and security, while all other
    /// mount nodes must be explicitly assigned a mountpoint to maintain structural integrity.
    fn new(fs: Arc<dyn FileSystem>, parent_mount: Option<Weak<Mount>>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: Dentry::new_root(fs.root_inode()),
            mountpoint: RwLock::new(None),
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
    pub(super) fn do_mount(
        self: &Arc<Self>,
        fs: Arc<dyn FileSystem>,
        mountpoint: &Arc<Dentry>,
    ) -> Result<Arc<Self>> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let key = mountpoint.key();
        let child_mount = Self::new(fs, Some(Arc::downgrade(self)));
        self.children.write().insert(key, child_mount.clone());
        child_mount.set_mountpoint(mountpoint);

        Ok(child_mount)
    }

    /// Unmounts a child mount node from the mountpoint and returns it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub(super) fn do_unmount(&self, mountpoint: &Dentry) -> Result<Arc<Self>> {
        let child_mount = self
            .children
            .write()
            .remove(&mountpoint.key())
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;

        child_mount.clear_mountpoint();

        Ok(child_mount)
    }

    /// Clones a mount node with the an root `Dentry`.
    ///
    /// The new mount node will have the same fs as the original one and
    /// have no parent and children. We should set the parent and children manually.
    fn clone_mount(&self, root_dentry: &Arc<Dentry>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: root_dentry.clone(),
            mountpoint: RwLock::new(None),
            parent: RwLock::new(None),
            children: RwLock::new(HashMap::new()),
            fs: self.fs.clone(),
            this: weak_self.clone(),
        })
    }

    /// Clones a mount tree starting from the specified root `Dentry`.
    ///
    /// The new mount tree will replicate the structure of the original tree.
    /// The new tree is a separate entity rooted at the given `Dentry`,
    /// and the original tree remains unchanged.
    ///
    /// If `recursive` is set to `true`, the entire tree will be copied.
    /// Otherwise, only the root mount node will be copied.
    pub(super) fn clone_mount_tree(&self, root_dentry: &Arc<Dentry>, recursive: bool) -> Arc<Self> {
        let new_root_mount = self.clone_mount(root_dentry);
        if !recursive {
            return new_root_mount;
        }

        let mut stack = vec![self.this()];
        let mut new_stack = vec![new_root_mount.clone()];
        while let Some(old_mount) = stack.pop() {
            let new_parent_mount = new_stack.pop().unwrap();
            let old_children = old_mount.children.read();
            for old_child_mount in old_children.values() {
                let mountpoint = old_child_mount.mountpoint().unwrap();
                if !mountpoint.is_descendant_of(old_mount.root_dentry()) {
                    continue;
                }
                let new_child_mount = old_child_mount.clone_mount(old_child_mount.root_dentry());
                let key = mountpoint.key();
                new_parent_mount
                    .children
                    .write()
                    .insert(key, new_child_mount.clone());
                new_child_mount.set_parent(&new_parent_mount);
                new_child_mount.set_mountpoint(&old_child_mount.mountpoint().unwrap());
                stack.push(old_child_mount.clone());
                new_stack.push(new_child_mount);
            }
        }

        new_root_mount
    }

    /// Detaches the mount node from the parent mount node.
    fn detach_from_parent(&self) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            let child = parent
                .children
                .write()
                .remove(&self.mountpoint().unwrap().key());

            if let Some(child) = child {
                child.clear_mountpoint();
            }
        }
    }

    /// Attaches the mount node to the mountpoint.
    fn attach_to_path(&self, target_path: &Path) {
        let key = target_path.key();
        target_path
            .mount_node()
            .children
            .write()
            .insert(key, self.this());
        self.set_parent(target_path.mount_node());
        self.set_mountpoint(&target_path.dentry);
    }

    /// Grafts the mount node tree to the mountpoint.
    pub(super) fn graft_mount_tree(&self, target_path: &Path) -> Result<()> {
        if target_path.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        self.detach_from_parent();
        self.attach_to_path(target_path);
        Ok(())
    }

    /// Gets a child mount node from the mountpoint if any.
    pub(super) fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        self.children.read().get(&mountpoint.key()).cloned()
    }

    /// Gets the root `Dentry` of this mount node.
    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    /// Gets the mountpoint `Dentry` of this mount node if any.
    pub(super) fn mountpoint(&self) -> Option<Arc<Dentry>> {
        self.mountpoint.read().clone()
    }

    /// Sets the mountpoint.
    pub(super) fn set_mountpoint(&self, dentry: &Arc<Dentry>) {
        let mut mountpoint = self.mountpoint.write();
        if let Some(mountpoint) = mountpoint.as_deref() {
            mountpoint.dec_mount_count();
        }

        dentry.inc_mount_count();
        *mountpoint = Some(dentry.clone());
    }

    /// Clears the mountpoint.
    pub(super) fn clear_mountpoint(&self) {
        let mut mountpoint = self.mountpoint.write();
        if let Some(mountpoint) = mountpoint.as_deref() {
            mountpoint.dec_mount_count();
        }

        *mountpoint = None;
    }

    /// Flushes all pending filesystem metadata and cached file data to the device.
    pub fn sync(&self) -> Result<()> {
        let children: Vec<Arc<Mount>> = {
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
    pub(super) fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    /// Sets the parent mount node.
    ///
    /// In some cases we may need to reset the parent of
    /// the created Mount, such as move mount.
    fn set_parent(&self, mount: &Arc<Mount>) {
        let mut parent = self.parent.write();
        *parent = Some(Arc::downgrade(mount));
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }
}

impl Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("root", &self.root_dentry)
            .field("mountpoint", &self.mountpoint)
            .field("fs", &self.fs)
            .finish()
    }
}
