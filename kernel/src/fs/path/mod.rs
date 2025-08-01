// SPDX-License-Identifier: MPL-2.0

//! Form file paths within and across FSes with dentries and mount points.

use core::time::Duration;

use inherit_methods_macro::inherit_methods;
pub use mount::MountNode;
use ostd::sync::{PreemptDisabled, RwMutexReadGuard, RwMutexWriteGuard};

use crate::{
    fs::{
        path::dentry::Dentry,
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

/// The global mount lock, protecting the atomicity and consistency of mount-related
/// operations, especially for operations across different mount namespace.
pub(super) type MountLock = SpinLock<()>;

pub(super) type MountLockGuard = SpinLockGuard<'static, (), PreemptDisabled>;

/// The global instance of the mount lock.
pub(super) static MOUNT_LOCK: MountLock = MountLock::new(());
/// A mutex lock that protects the mount tree topology of a mount namespace.
///
/// # TODO: Maintains per-namespace instantiation after mount namespace mechanism
/// achieved.
pub(super) static NAMESPACE_LOCK: RwMutex<()> = RwMutex::new(());

/// Acquires a read lock on the mount tree, returning a guard.
fn begin_mount_tree_read() -> RwMutexReadGuard<'static, ()> {
    NAMESPACE_LOCK.read()
}

/// Acquires a write lock on the mount tree, returning a guard.
fn begin_mount_tree_write() -> RwMutexWriteGuard<'static, ()> {
    NAMESPACE_LOCK.write()
}

/// A `Path` is used to represent an exact location in the VFS tree.
#[derive(Debug, Clone)]
pub struct Path {
    mount_node: Arc<MountNode>,
    dentry: Arc<Dentry>,
}

impl Path {
    /// Creates a new `Path` to represent the root directory of a file system.
    pub fn new_fs_root(mount_node: Arc<MountNode>) -> Self {
        let inner = mount_node.root_dentry().clone();
        Self::new(mount_node, inner)
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
        Ok(Self::new(self.mount_node.clone(), new_child_dentry))
    }

    fn new(mount_node: Arc<MountNode>, dentry: Arc<Dentry>) -> Self {
        Self { mount_node, dentry }
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

        let _read_guard = begin_mount_tree_read();

        let target_path = if is_dot(name) {
            self.this()
        } else if is_dotdot(name) {
            self.effective_parent().unwrap_or_else(|| self.this())
        } else {
            let target_inner_opt = self.dentry.lookup_via_cache(name)?;
            match target_inner_opt {
                Some(target_inner) => Self::new(self.mount_node.clone(), target_inner),
                None => {
                    let target_inner = self.dentry.lookup_via_fs(name)?;
                    Self::new(self.mount_node.clone(), target_inner)
                }
            }
        };

        Ok(target_path.top_path())
    }

    /// Gets the absolute path.
    ///
    /// It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path = self.effective_name();
        let mut current_dir = self.this();

        while let Some(parent_dir) = current_dir.effective_parent() {
            path = {
                let parent_name = parent_dir.effective_name();
                if parent_name != "/" {
                    parent_name + "/" + &path
                } else {
                    parent_name + &path
                }
            };
            current_dir = parent_dir;
        }

        debug_assert!(path.starts_with('/'));
        path
    }

    /// Gets the effective name of the `Path`.
    ///
    /// If it is the root of a mount, it will go up to the mountpoint
    /// to get the name of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.dentry.is_mount_root() {
            return self.dentry.name();
        }

        let Some(parent) = self.mount_node.parent() else {
            return self.dentry.name();
        };
        let Some(mountpoint) = self.mount_node.mountpoint() else {
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
            return Some(Self::new(
                self.mount_node.clone(),
                self.dentry.parent().unwrap(),
            ));
        }

        let parent = self.mount_node.parent()?;
        let mountpoint = self.mount_node.mountpoint()?;

        let mount_parent = Self::new(parent.upgrade().unwrap(), mountpoint);
        mount_parent.effective_parent()
    }

    /// Gets the top `Path` of the current.
    ///
    /// Used when different file systems are mounted on the same mount point.
    ///
    /// For example, first `mount /dev/sda1 /mnt` and then `mount /dev/sda2 /mnt`.
    /// After the second mount is completed, the content of the first mount will be overridden.
    /// We need to recursively obtain the top `Path`.
    fn top_path(self) -> Self {
        let mut current_path = self;
        while current_path.dentry.is_mountpoint() {
            let Path { mount_node, dentry } = current_path;
            current_path = {
                let child_mount = mount_node.find_child_mount(&dentry).unwrap();
                let dentry = child_mount.root_dentry().clone();
                Self::new(child_mount, dentry)
            };
        }

        current_path
    }

    /// Mounts the fs on current `Path` as a mountpoint.
    ///
    /// If the given mountpoint has already been mounted,
    /// its mounted child mount will be updated.
    /// The root Path cannot be mounted.
    ///
    /// Returns the mounted child mount.
    pub fn mount(&self, fs: Arc<dyn FileSystem>) -> Result<Arc<MountNode>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.effective_parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "can not mount on root");
        }

        let _write_guard = begin_mount_tree_write();

        let child_mount = self.mount_node.new_child(fs);

        let mount_lock = MOUNT_LOCK.lock();
        self.mount_node.mount(child_mount.clone(), &mount_lock);
        child_mount.set_mountpoint(self.dentry.clone());

        Ok(child_mount)
    }

    /// Unmounts and returns the mounted child mount.
    ///
    /// Note that the root mount cannot be unmounted.
    pub fn unmount(&self) -> Result<Arc<MountNode>> {
        if !self.dentry.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "not mounted");
        }

        let Some(mountpoint) = self.mount_node.mountpoint() else {
            return_errno_with_message!(Errno::EINVAL, "cannot umount root mount");
        };

        let _write_guard = begin_mount_tree_write();

        let parent_mount_node = self.mount_node.parent().unwrap().upgrade().unwrap();

        let mount_lock = MOUNT_LOCK.lock();
        let old_mount = parent_mount_node
            .unmount(&mountpoint, &mount_lock)
            .ok_or(Error::new(Errno::EINVAL))?;
        self.dentry.sub_mount();

        Ok(old_mount)
    }

    /// Creates a `Path` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Self> {
        let inner = self.dentry.mknod(name, mode, type_)?;
        Ok(Self::new(self.mount_node.clone(), inner))
    }

    /// Links a new name for the `Path`.
    pub fn link(&self, old: &Self, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&old.mount_node, &self.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }

        self.dentry.link(&old.dentry, name)
    }

    /// Deletes a `Path`.
    pub fn unlink(&self, name: &str) -> Result<()> {
        self.dentry.unlink(name)
    }

    /// Deletes a directory `Path`.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        self.dentry.rmdir(name)
    }

    /// Renames a `Path` to the new `Path` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Self, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.mount_node, &new_dir.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }

        self.dentry.rename(old_name, &new_dir.dentry, new_name)
    }

    /// Binds mount the `Path` to the destination `Path`.
    ///
    /// If `recursive` is true, it will bind mount the whole mount tree
    /// to the destination `Path`. Otherwise, it will only bind mount
    /// the root mount node.
    pub fn bind_mount_to(&self, dst_dentry: &Self, recursive: bool) -> Result<()> {
        let _write_guard = begin_mount_tree_write();
        let new_mount = self
            .mount_node
            .clone_mount_node_tree(&self.dentry, recursive);
        new_mount.graft_mount_node_tree(dst_dentry)?;
        Ok(())
    }

    fn this(&self) -> Self {
        self.clone()
    }

    /// Gets the mount node of current `Path`.
    pub fn mount_node(&self) -> &Arc<MountNode> {
        &self.mount_node
    }
}

#[inherit_methods(from = "self.dentry")]
impl Path {
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
