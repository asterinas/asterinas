// SPDX-License-Identifier: MPL-2.0

//! Form file paths within and across FSes with dentries and mount points.

use core::time::Duration;

use inherit_methods_macro::inherit_methods;
pub use mount::{Mount, MountPropType, PerMountFlags};
pub use mount_namespace::MountNamespace;
pub use resolver::{AT_FDCWD, AbsPathResult, FsPath, LookupResult, PathResolver, SplitPath};

use crate::{
    fs::{
        inode_handle::InodeHandle,
        path::dentry::Dentry,
        utils::{
            CreationFlags, FileSystem, FsFlags, Inode, InodeMode, InodeType, Metadata, MknodType,
            OpenArgs, Permission, StatusFlags, XattrName, XattrNamespace, XattrSetFlags,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

mod dentry;
mod mount;
mod mount_namespace;
mod resolver;

/// A `Path` is used to represent an exact location in the VFS tree.
///
/// Each `Path` corresponds to a node in the VFS tree, and a single node
/// may have multiple `Path` instances referencing it due to mount operations.
#[derive(Debug, Clone)]
pub struct Path {
    mount: Arc<Mount>,
    dentry: Arc<Dentry>,
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.mount.id() == other.mount.id() && Arc::ptr_eq(&self.dentry, &other.dentry)
    }
}

impl Eq for Path {}

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

    /// Creates a new pseudo `Path`.
    pub(super) fn new_pseudo(
        mount: Arc<Mount>,
        inode: Arc<dyn Inode>,
        name_fn: fn(&dyn Inode) -> String,
    ) -> Self {
        let dentry = Dentry::new_pseudo(inode, name_fn);
        Self::new(mount, dentry)
    }

    fn new(mount: Arc<Mount>, dentry: Arc<Dentry>) -> Self {
        Self { mount, dentry }
    }

    /// Gets the mount node of current `Path`.
    pub fn mount_node(&self) -> &Arc<Mount> {
        &self.mount
    }

    /// Returns true if the current `Path` is the root of its mount.
    pub fn is_mount_root(&self) -> bool {
        Arc::ptr_eq(&self.dentry, self.mount.root_dentry())
    }

    /// Opens the `Path` with the given `OpenArgs`.
    ///
    /// Returns an `InodeHandle` on success.
    pub fn open(&self, open_args: OpenArgs) -> Result<InodeHandle> {
        let inode = self.inode().as_ref();
        let inode_type = inode.type_();
        let creation_flags = &open_args.creation_flags;
        let status_flags = &open_args.status_flags;

        if inode_type == InodeType::SymLink
            && creation_flags.contains(CreationFlags::O_NOFOLLOW)
            && !status_flags.contains(StatusFlags::O_PATH)
        {
            return_errno_with_message!(Errno::ELOOP, "the file is a symlink");
        }

        if creation_flags.contains(CreationFlags::O_CREAT)
            && creation_flags.contains(CreationFlags::O_EXCL)
        {
            return_errno_with_message!(Errno::EEXIST, "the file already exists");
        }
        if creation_flags.contains(CreationFlags::O_DIRECTORY) && inode_type != InodeType::Dir {
            return_errno_with_message!(
                Errno::ENOTDIR,
                "O_DIRECTORY is specified but the file is not a directory"
            );
        }

        if inode_type.is_regular_file()
            && creation_flags.contains(CreationFlags::O_TRUNC)
            && !status_flags.contains(StatusFlags::O_PATH)
        {
            self.resize(0)?;
        }

        InodeHandle::new(self.clone(), open_args.access_mode, *status_flags)
    }

    /// Gets the real name of the `Path` from its dentry.
    pub fn name(&self) -> String {
        self.dentry.name()
    }

    /// Gets the parent `Path` within the same mount.
    ///
    /// This method returns the parent path within the same filesystem/mount.
    /// It does NOT cross mount boundaries. If the current path is the root of a mount,
    /// it will return `None`.
    ///
    /// For cross-filesystem parent lookup, use `effective_parent()` instead.
    pub(super) fn parent_within_mount(&self) -> Option<Self> {
        let parent = self.dentry.parent()?;
        Some(Self::new(self.mount.clone(), parent))
    }

    /// Gets the top `Path` of the current.
    ///
    /// Used when different file systems are mounted on the same mount point.
    ///
    /// For example, first `mount /dev/sda1 /mnt` and then `mount /dev/sda2 /mnt`.
    /// After the second mount is completed, the content of the first mount will be overridden.
    /// We need to recursively obtain the top `Path`.
    pub(super) fn get_top_path(mut self) -> Self {
        while self.dentry.is_mountpoint() {
            if let Some(child_mount) = self.mount.get(&self.dentry) {
                let inner = child_mount.root_dentry().clone();
                self = Self::new(child_mount, inner);
            } else {
                break;
            }
        }

        self
    }

    /// Finds the corresponding `Path` in the given mount namespace.
    pub(super) fn find_corresponding_mount(&self, mnt_ns: &Arc<MountNamespace>) -> Option<Self> {
        let corresponding_mount = self.mount.find_corresponding_mount(mnt_ns)?;
        let corresponding_path = Self::new(corresponding_mount, self.dentry.clone());

        Some(corresponding_path)
    }

    /// Returns true if the `Path` represents a pseudo file.
    fn is_pseudo(&self) -> bool {
        self.dentry.is_pseudo()
    }

    fn this(&self) -> Self {
        self.clone()
    }
}

impl Path {
    /// Mounts a filesystem at the current path.
    ///
    /// This method attaches a given filesystem to the VFS tree at the location
    /// represented by `self`. The current path becomes the mountpoint for the new
    /// filesystem.
    ///
    /// Returns the newly created child mount on success.
    ///
    /// # Errors
    ///
    /// Returns `ENOTDIR` if the path is not a directory.
    /// Returns `EINVAL` if attempting to mount on root or if the path is not
    /// in the current mount namespace.
    pub fn mount(
        &self,
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        ctx: &Context,
    ) -> Result<Arc<Mount>> {
        if self.type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the path is not a directory");
        }

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();

        if path_resolver.root() == self {
            return_errno_with_message!(Errno::EINVAL, "the root cannot be mounted on");
        }

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(Errno::EINVAL, "the path is not in this mount namespace");
        }

        let child_mount = self.mount.do_mount(fs, flags, &self.dentry)?;

        Ok(child_mount)
    }

    /// Unmounts the filesystem mounted at the current path.
    ///
    /// Returns the unmounted child mount on success.
    ///
    /// # Errors
    ///
    /// Returns `EINVAL` in the following cases:
    /// - The current path is not a mount root.
    /// - The mount of the current path is the root mount.
    /// - The current path is not in the current mount namespace.
    pub fn unmount(&self, ctx: &Context) -> Result<Arc<Mount>> {
        if !self.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "the path is not a mount root");
        }

        let Some(mountpoint) = self.mount.mountpoint() else {
            return_errno_with_message!(Errno::EINVAL, "the root mount cannot be unmounted");
        };

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(Errno::EINVAL, "the path is not in this mount namespace");
        }

        let parent_mount = self.mount.parent().unwrap().upgrade().unwrap();
        let child_mount = parent_mount.do_unmount(&mountpoint)?;

        Ok(child_mount)
    }

    /// Remounts the filesystem with new `PerMountFlags` and optionally new `FsFlags`.
    ///
    /// If `fs_flags` is provided, it will update the flags of the mounted filesystem,
    /// otherwise, only the flags of the current mount will be updated.
    ///
    /// # Errors
    ///
    /// Returns `EINVAL` in the following cases:
    /// - The current path is not a mount root.
    /// - The current path is not in the current mount namespace.
    pub fn remount(
        &self,
        mount_flags: PerMountFlags,
        fs_flags: Option<FsFlags>,
        data: Option<CString>,
        ctx: &Context,
    ) -> Result<()> {
        if !self.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "the path is not a mount root");
        };

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(Errno::EINVAL, "the path is not in this mount namespace");
        }

        self.mount.remount(mount_flags, fs_flags, data, ctx)
    }

    /// Creates a bind mount from the current path to the destination path.
    ///
    /// Creates a new mount tree that mirrors either the root mount (non-recursive)
    /// or the entire mount subtree (recursive), and attaches it to the destination path.
    ///
    /// # Errors
    ///
    /// Returns `ENOTDIR` if the `dst_path` is not a directory.
    /// Returns `EINVAL` if either source or destination path is not in the
    /// current mount namespace.
    pub fn bind_mount_to(&self, dst_path: &Self, recursive: bool, ctx: &Context) -> Result<()> {
        let can_bind = {
            let src_is_dir = self.type_() == InodeType::Dir;
            let dst_is_dir = dst_path.type_() == InodeType::Dir;
            (src_is_dir && dst_is_dir) || (!src_is_dir && !dst_is_dir)
        };
        if !can_bind {
            return_errno_with_message!(
                Errno::ENOTDIR,
                "the source and destination must both be directories or both be non-directories"
            );
        }

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the source path is not in this mount namespace"
            );
        }
        if !current_mnt_ns.owns(&dst_path.mount) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the destination path is not in this mount namespace"
            );
        }

        let new_mount = self.mount.clone_mount_tree(&self.dentry, None, recursive);
        new_mount.graft_mount_tree(dst_path)?;
        Ok(())
    }

    /// Moves a mount tree from the current path to the destination path.
    ///
    /// # Errors
    ///
    /// Returns `ENOTDIR` if the `dst_path` is not a directory.
    /// Returns `EINVAL` in the following cases:
    /// - The current path is not a mount root.
    /// - The mount of the current path is the root mount.
    /// - Either source or destination path is not in the current mount namespace
    pub fn move_mount_to(&self, dst_path: &Self, ctx: &Context) -> Result<()> {
        if !self.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "the path is not a mount root");
        };
        if self.mount_node().parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "the root mount can not be moved");
        }

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the source path is not in this mount namespace"
            );
        }
        if !current_mnt_ns.owns(&dst_path.mount) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the destination path is not in this mount namespace"
            );
        }

        self.mount.graft_mount_tree(dst_path)
    }

    /// Sets the propagation type of the mount of this `Path`.
    pub fn set_mount_propagation(
        &self,
        prop: MountPropType,
        recursive: bool,
        ctx: &Context,
    ) -> Result<()> {
        if !self.is_mount_root() {
            return_errno_with_message!(Errno::EINVAL, "the path is not a mount root");
        };

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
        if !current_mnt_ns.owns(&self.mount) {
            return_errno_with_message!(Errno::EINVAL, "the path is not in this mount namespace");
        }

        self.mount.set_propagation(prop, recursive);

        Ok(())
    }
}

// Methods inherited from `Dentry`.
#[inherit_methods(from = "self.dentry")]
impl Path {
    pub fn inode(&self) -> &Arc<dyn Inode>;
    pub fn type_(&self) -> InodeType;
    pub fn unlink(&self, name: &str) -> Result<()>;
    pub fn rmdir(&self, name: &str) -> Result<()>;

    /// Creates a `Path` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Self> {
        let inner = self.dentry.mknod(name, mode, type_)?;
        Ok(Self::new(self.mount.clone(), inner))
    }

    /// Links a new name for the `Path`.
    pub fn link(&self, old: &Self, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&old.mount, &self.mount) {
            return_errno_with_message!(Errno::EXDEV, "the operation cannot cross mounts");
        }

        self.dentry.link(&old.dentry, name)
    }

    /// Renames a `Path` to the new `Path` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Self, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.mount, &new_dir.mount) {
            return_errno_with_message!(Errno::EXDEV, "the operation cannot cross mounts");
        }

        self.dentry.rename(old_name, &new_dir.dentry, new_name)
    }
}

// Methods inherited from `Inode`.
#[inherit_methods(from = "self.inode()")]
impl Path {
    pub fn fs(&self) -> Arc<dyn FileSystem>;
    pub fn sync_all(&self) -> Result<()>;
    pub fn sync_data(&self) -> Result<()>;
    pub fn metadata(&self) -> Metadata;
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

pub(super) fn init() {
    mount::init();
}
