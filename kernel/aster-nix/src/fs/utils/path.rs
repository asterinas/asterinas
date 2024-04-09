// SPDX-License-Identifier: MPL-2.0

use super::{Dentry, FileSystem, InodeType, MountNode, NAME_MAX};
use crate::prelude::*;

/// The Path can represent a location in the mount tree.
#[derive(Debug)]
pub struct Path {
    mount_node: Arc<MountNode>,
    dentry: Arc<Dentry>,
    this: Weak<Path>,
}

impl Path {
    pub fn new(mount_node: Arc<MountNode>, dentry: Arc<Dentry>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            mount_node,
            dentry,
            this: weak_self.clone(),
        })
    }

    /// Lookup a path.
    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if self.dentry.inode().type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if !self.dentry.inode().mode()?.is_executable() {
            return_errno!(Errno::EACCES);
        }
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let path = match name {
            "." => self.this(),
            ".." => self.effective_parent().unwrap_or(self.this()),
            name => {
                let children_dentry = self.dentry.lookup_fast(name);
                match children_dentry {
                    Some(dentry) => Path::new(self.mount_node().clone(), dentry.clone()),
                    None => {
                        let slow_dentry = self.dentry.lookup_slow(name)?;
                        Path::new(self.mount_node().clone(), slow_dentry.clone())
                    }
                }
            }
        };
        let path = path.overlaid_path();
        Ok(path)
    }

    // Get the absolute path.
    //
    // It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path = self.effective_name();
        let mut dir_path = self.this();

        loop {
            match dir_path.effective_parent() {
                None => break,
                Some(parent_dir_path) => {
                    path = {
                        let parent_name = parent_dir_path.effective_name();
                        if parent_name != "/" {
                            parent_name + "/" + &path
                        } else {
                            parent_name + &path
                        }
                    };
                    dir_path = parent_dir_path;
                }
            }
        }
        debug_assert!(path.starts_with('/'));
        path
    }

    /// Get the effective name of path.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the name
    /// of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.dentry.is_root_of_mount() {
            return self.dentry.name();
        }

        if self.mount_node.parent().is_some() & self.mount_node.mountpoint_dentry().is_some() {
            let parent_path = Path::new(
                self.mount_node.parent().unwrap().upgrade().unwrap().clone(),
                self.mount_node.mountpoint_dentry().unwrap().clone(),
            );
            parent_path.effective_name()
        } else {
            self.dentry.name()
        }
    }

    /// Get the effective parent of path.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the parent
    /// of the mountpoint recursively.
    fn effective_parent(&self) -> Option<Arc<Self>> {
        if !self.dentry.is_root_of_mount() {
            return Some(Path::new(
                self.mount_node.clone(),
                self.dentry.parent().unwrap().clone(),
            ));
        }
        if self.mount_node.parent().is_some() & self.mount_node.mountpoint_dentry().is_some() {
            let parent_path = Path::new(
                self.mount_node.parent().unwrap().upgrade().unwrap().clone(),
                self.mount_node.mountpoint_dentry().unwrap().clone(),
            );
            parent_path.effective_parent()
        } else {
            None
        }
    }

    /// Get the overlaid path of self.
    ///
    /// It will jump into the child mount if it is a mountpoint.
    fn overlaid_path(&self) -> Arc<Self> {
        if !self.dentry.is_mountpoint() {
            return self.this();
        }
        match self.mount_node.get(self) {
            Some(child_mount) => {
                Path::new(child_mount.clone(), child_mount.root_dentry().clone()).overlaid_path()
            }
            None => self.this(),
        }
    }

    /// Mount the fs on this path. It will make this path's dentry to be a mountpoint.
    ///
    /// If the given mountpoint has already been mounted, then its mounted child mount
    /// will be updated.
    /// The root dentry cannot be mounted.
    ///
    /// Return the mounted child mount.
    pub fn mount(&self, fs: Arc<dyn FileSystem>) -> Result<Arc<MountNode>> {
        if self.dentry.inode().type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.effective_parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "can not mount on root");
        }
        let child_mount = self.mount_node().mount(fs, &self.this())?;
        self.dentry().set_mountpoint();
        Ok(child_mount)
    }

    /// Unmount and return the mounted child mount.
    ///
    /// Note that the root mount cannot be unmounted.
    pub fn umount(&self) -> Result<Arc<MountNode>> {
        if !self.dentry.is_root_of_mount() {
            return_errno_with_message!(Errno::EINVAL, "not mounted");
        }

        let mount_node = self.mount_node.clone();
        let Some(mountpoint_dentry) = mount_node.mountpoint_dentry() else {
            return_errno_with_message!(Errno::EINVAL, "cannot umount root mount");
        };

        let mountpoint_mount_node = mount_node.parent().unwrap().upgrade().unwrap();
        let mountpoint_path = Path::new(mountpoint_mount_node.clone(), mountpoint_dentry.clone());

        let child_mount = mountpoint_mount_node.umount(&mountpoint_path)?;
        mountpoint_dentry.clear_mountpoint();
        Ok(child_mount)
    }

    /// Link a new name for the path's dentry by linking inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.dentry.inode().type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let children = self.dentry.lookup_fast(name);
        if children.is_some() {
            return_errno!(Errno::EEXIST);
        }
        if !Arc::ptr_eq(old.mount_node(), self.mount_node()) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }
        let old_inode = old.dentry.inode();
        self.dentry.inode().link(old_inode, name)?;
        let dentry = self.dentry.new_child(old_inode.clone(), name);
        self.dentry.insert_dentry(&dentry);
        Ok(())
    }

    /// Get the arc reference to self.
    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Get the mount node of this path.
    pub fn mount_node(&self) -> &Arc<MountNode> {
        &self.mount_node
    }

    /// Get the dentry of this path.
    pub fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }
}
