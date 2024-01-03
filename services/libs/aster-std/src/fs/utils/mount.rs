// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

use super::{Dentry, DentryKey, FileSystem, InodeType};

/// The MountNode can form a mount tree to maintain the mount information.
pub struct MountNode {
    /// Root dentry.
    root_dentry: Arc<Dentry>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint_dentry: Option<Arc<Dentry>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// Child mount nodes which are mounted on one dentry of self.
    children: Mutex<BTreeMap<DentryKey, Arc<Self>>>,
    /// Reference to self.
    this: Weak<Self>,
}

impl MountNode {
    /// Create a root mount node with an associated FS.
    ///
    /// The root mount node is not mounted on other mount nodes(which means it has no
    /// parent). The root inode of the fs will form the root dentry of it.
    ///
    /// It is allowed to create a mount node even if the fs has been provided to another
    /// mount node. It is the fs's responsibility to ensure the data consistency.
    pub fn new_root(fs: Arc<dyn FileSystem>) -> Arc<Self> {
        Self::new(fs, None)
    }

    /// The internal constructor.
    ///
    /// Root mount node has no mountpoint which other mount nodes must have mountpoint.
    fn new(fs: Arc<dyn FileSystem>, mountpoint: Option<Arc<Dentry>>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            root_dentry: Dentry::new_root(fs.root_inode(), weak_self.clone()),
            mountpoint_dentry: mountpoint,
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
        if !Arc::ptr_eq(&mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }
        if mountpoint.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let key = mountpoint.key();
        let child_mount = Self::new(fs, Some(mountpoint.clone()));
        self.children.lock().insert(key, child_mount.clone());
        Ok(child_mount)
    }

    /// Unmount a child mount node from the mountpoint and return it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub fn umount(&self, mountpoint: &Dentry) -> Result<Arc<Self>> {
        if !Arc::ptr_eq(&mountpoint.mount_node(), &self.this()) {
            return_errno_with_message!(Errno::EINVAL, "mountpoint not belongs to this");
        }

        let child_mount = self
            .children
            .lock()
            .remove(&mountpoint.key())
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;
        Ok(child_mount)
    }

    /// Try to get a child mount node from the mountpoint.
    pub fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        if !Arc::ptr_eq(&mountpoint.mount_node(), &self.this()) {
            return None;
        }
        self.children.lock().get(&mountpoint.key()).cloned()
    }

    /// Get the root dentry of this mount node.
    pub fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    /// Try to get the mountpoint dentry of this mount node.
    pub fn mountpoint_dentry(&self) -> Option<&Arc<Dentry>> {
        self.mountpoint_dentry.as_ref()
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
    pub fn parent(&self) -> Option<Arc<Self>> {
        self.mountpoint_dentry
            .as_ref()
            .map(|dentry| dentry.mount_node())
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
