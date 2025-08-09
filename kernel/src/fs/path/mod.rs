// SPDX-License-Identifier: MPL-2.0

//! Form file paths within and across FSes with dentries and mount points.

use core::time::Duration;

use inherit_methods_macro::inherit_methods;
pub use mount::Mount;

use crate::{
    fs::{
        path::dentry::{Dentry, DentryKey},
        utils::{
            FileSystem, Inode, InodeMode, InodeType, Metadata, MknodType, Permission, XattrName,
            XattrNamespace, XattrSetFlags, NAME_MAX,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

mod dentry;
mod mount;

/// A `Path` is used to represent an exact location in the VFS tree.
///
/// Each `Path` corresponds to a node in the VFS tree, and a single node
/// may have multiple `Path` instances referencing it due to mount operations.
#[derive(Debug, Clone)]
pub struct Path {
    mount: Arc<Mount>,
    dentry: Arc<Dentry>,
}

impl Path {
    /// Creates a new `Path` to represent the root directory of a file system.
    pub fn new_fs_root(mount: Arc<Mount>) -> Self {
        let inner = mount.root_dentry().clone();
        Self::new(mount, inner)
    }

    /// Creates a new `Path` to represent the child directory of a file system.
    pub fn new_fs_child(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Self> {
        if self
            .inode()
            .check_permission(Permission::MAY_WRITE)
            .is_err()
        {
            return_errno!(Errno::EACCES);
        }
        let new_child_dentry = self.dentry.create(name, type_, mode)?;
        Ok(Self::new(self.mount.clone(), new_child_dentry))
    }

    fn new(mount: Arc<Mount>, dentry: Arc<Dentry>) -> Self {
        Self { mount, dentry }
    }

    /// Gets the mount node of current `Path`.
    pub fn mount_node(&self) -> &Arc<Mount> {
        &self.mount
    }

    /// Lookups the target `Path` given the `name`.
    pub fn lookup(&self, name: &str) -> Result<Self> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.inode().check_permission(Permission::MAY_EXEC).is_err() {
            return_errno!(Errno::EACCES);
        }
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let target_path = if is_dot(name) {
            self.this()
        } else if is_dotdot(name) {
            self.effective_parent().unwrap_or_else(|| self.this())
        } else {
            let target_inner_opt = self.dentry.lookup_via_cache(name)?;
            match target_inner_opt {
                Some(target_inner) => Self::new(self.mount.clone(), target_inner),
                None => {
                    let target_inner = self.dentry.lookup_via_fs(name)?;
                    Self::new(self.mount.clone(), target_inner)
                }
            }
        };

        Ok(target_path.get_top_path())
    }

    /// Gets the absolute path.
    ///
    /// It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path_name = self.effective_name();
        let mut current_dir = self.this();

        while let Some(parent_dir) = current_dir.effective_parent() {
            path_name = {
                let parent_name = parent_dir.effective_name();
                if parent_name != "/" {
                    parent_name + "/" + &path_name
                } else {
                    parent_name + &path_name
                }
            };
            current_dir = parent_dir;
        }

        debug_assert!(path_name.starts_with('/'));
        path_name
    }

    /// Gets the effective name of the `Path`.
    ///
    /// If it is the root of a mount, it will go up to the mountpoint
    /// to get the name of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.dentry.is_mount_root() {
            return self.dentry.name();
        }

        let Some(parent) = self.mount.parent() else {
            return self.dentry.name();
        };
        let Some(mountpoint) = self.mount.mountpoint() else {
            return self.dentry.name();
        };

        let mount_parent = Self::new(parent.upgrade().unwrap(), mountpoint);
        mount_parent.effective_name()
    }

    /// Gets the effective parent of the `Path`.
    ///
    /// If it is the root of a mount, it will go up to the mountpoint
    /// to get the parent of the mountpoint recursively.
    fn effective_parent(&self) -> Option<Self> {
        if !self.dentry.is_mount_root() {
            return Some(Self::new(self.mount.clone(), self.dentry.parent().unwrap()));
        }

        let parent = self.mount.parent()?;
        let mountpoint = self.mount.mountpoint()?;

        let mount_parent = Self::new(parent.upgrade().unwrap(), mountpoint);
        mount_parent.effective_parent()
    }

    /// Gets the top `Dentry` of the current.
    ///
    /// Used when different file systems are mounted on the same mount point.
    ///
    /// For example, first `mount /dev/sda1 /mnt` and then `mount /dev/sda2 /mnt`.
    /// After the second mount is completed, the content of the first mount will be overridden.
    /// We need to recursively obtain the top `Dentry`.
    fn get_top_path(self) -> Self {
        if !self.dentry.is_mountpoint() {
            return self;
        }

        match self.mount.get(&self.dentry) {
            Some(child_mount) => {
                let inner = child_mount.root_dentry().clone();
                Self::new(child_mount, inner).get_top_path()
            }
            None => self,
        }
    }

    fn this(&self) -> Self {
        self.clone()
    }
}

impl Path {
    /// Mounts the fs on current `Dentry` as a mountpoint.
    ///
    /// If the given mountpoint has already been mounted,
    /// its mounted child mount will be updated.
    /// The root Dentry cannot be mounted.
    ///
    /// Returns the mounted child mount.
    pub fn mount(&self, fs: Arc<dyn FileSystem>) -> Result<Arc<Mount>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.effective_parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "can not mount on root");
        }

        let child_mount = self.mount.do_mount(fs, &self.dentry)?;

        Ok(child_mount)
    }

    /// Unmounts and returns the mounted child mount.
    ///
    /// Note that the root mount cannot be unmounted.
    pub fn unmount(&self) -> Result<Arc<Mount>> {
        if !self.dentry.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "not mounted");
        }

        let Some(mountpoint) = self.mount.mountpoint() else {
            return_errno_with_message!(Errno::EINVAL, "cannot umount root mount");
        };

        let parent_mount = self.mount.parent().unwrap().upgrade().unwrap();

        let child_mount = parent_mount.do_unmount(&mountpoint)?;

        Ok(child_mount)
    }

    /// Binds mount of the current `Path` to the destination `Path`.
    ///
    /// This operation will creates a new mount node tree that mirrors either
    /// just the root (non-recursive) or the entire mount subtree (recursive),
    /// and grafts it to the destination `Path`.
    pub fn bind_mount_to(&self, dst_path: &Self, recursive: bool) -> Result<()> {
        let new_mount = self.mount.clone_mount_tree(&self.dentry, recursive);
        new_mount.graft_mount_tree(dst_path)?;
        Ok(())
    }

    /// Moves the mount tree rooted at the current `Path` to the destination `Path`.
    ///
    /// If the current path is not a mount root or is a root mount, returns `Err`.
    pub fn move_mount_to(&self, dst_path: &Self) -> Result<()> {
        if !self.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "The current path is not a mount root");
        };
        if self.mount_node().parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "The root mount can not be moved");
        }

        self.mount.graft_mount_tree(dst_path)
    }

    /// Creates a `Path` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Self> {
        let inner = self.dentry.mknod(name, mode, type_)?;
        Ok(Self::new(self.mount.clone(), inner))
    }

    /// Links a new name for the `Path`.
    pub fn link(&self, old: &Self, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&old.mount, &self.mount) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }

        self.dentry.link(&old.dentry, name)
    }

    /// Renames a `Path` to the new `Path` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Self, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.mount, &new_dir.mount) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }

        self.dentry.rename(old_name, &new_dir.dentry, new_name)
    }
}

#[inherit_methods(from = "self.dentry")]
impl Path {
    pub fn unlink(&self, name: &str) -> Result<()>;
    pub fn rmdir(&self, name: &str) -> Result<()>;
    pub fn fs(&self) -> Arc<dyn FileSystem>;
    pub fn sync_all(&self) -> Result<()>;
    pub fn sync_data(&self) -> Result<()>;
    pub fn metadata(&self) -> Metadata;
    pub fn type_(&self) -> InodeType;
    pub fn mode(&self) -> Result<InodeMode>;
    pub fn set_mode(&self, mode: InodeMode) -> Result<()>;
    pub fn size(&self) -> usize;
    pub fn resize(&self, size: usize) -> Result<()>;
    pub fn owner(&self) -> Result<Uid>;
    pub fn set_owner(&self, uid: Uid) -> Result<()>;
    pub fn group(&self) -> Result<Gid>;
    pub fn set_group(&self, gid: Gid) -> Result<()>;
    pub fn atime(&self) -> Duration;
    pub fn set_atime(&self, time: Duration);
    pub fn mtime(&self) -> Duration;
    pub fn set_mtime(&self, time: Duration);
    pub fn ctime(&self) -> Duration;
    pub fn set_ctime(&self, time: Duration);
    pub fn key(&self) -> DentryKey;
    pub fn inode(&self) -> &Arc<dyn Inode>;
    pub fn is_mount_root(&self) -> bool;
    pub fn is_mountpoint(&self) -> bool;
    pub fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()>;
    pub fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize>;
    pub fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize>;
    pub fn remove_xattr(&self, name: XattrName) -> Result<()>;
}

/// Checks if the file name is ".", indicating it's the current directory.
pub const fn is_dot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    name_bytes.len() == 1 && name_bytes[0] == DOT_BYTE
}

/// Checks if the file name is "..", indicating it's the parent directory.
pub const fn is_dotdot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    name_bytes.len() == 2 && name_bytes[0] == DOT_BYTE && name_bytes[1] == DOT_BYTE
}

/// Checks if the file name is "." or "..", indicating it's the current or parent directory.
pub const fn is_dot_or_dotdot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    let name_len = name_bytes.len();
    if name_len == 1 {
        name_bytes[0] == DOT_BYTE
    } else if name_len == 2 {
        name_bytes[0] == DOT_BYTE && name_bytes[1] == DOT_BYTE
    } else {
        false
    }
}

const DOT_BYTE: u8 = b'.';
