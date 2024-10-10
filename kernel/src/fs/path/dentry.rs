// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use hashbrown::HashMap;
use inherit_methods_macro::inherit_methods;
use ostd::sync::RwMutexWriteGuard;

use crate::{
    fs::{
        path::mount::MountNode,
        utils::{FileSystem, Inode, InodeMode, InodeType, Metadata, MknodType, NAME_MAX},
    },
    prelude::*,
    process::{Gid, Uid},
};

/// A `Dentry` is used to represent a location in the mount tree.
#[derive(Debug, Clone)]
pub struct Dentry {
    mount_node: Arc<MountNode>,
    inner: Arc<Dentry_>,
}

/// The inner structure of `Dentry` for caching helpful nodes
/// to accelerate the path lookup.
pub struct Dentry_ {
    inode: Arc<dyn Inode>,
    name_and_parent: RwLock<Option<(String, Arc<Dentry_>)>>,
    children: RwMutex<Children>,
    flags: AtomicU32,
    this: Weak<Dentry_>,
}

impl Dentry_ {
    /// Creates a new root `Dentry_` with the given inode.
    ///
    /// It is been created during the construction of the `MountNode`.
    /// The `MountNode` holds an arc reference to this root `Dentry_`.
    pub(super) fn new_root(inode: Arc<dyn Inode>) -> Arc<Self> {
        Self::new(inode, DentryOptions::Root)
    }

    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            inode,
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            name_and_parent: match options {
                DentryOptions::Leaf(name_and_parent) => RwLock::new(Some(name_and_parent)),
                _ => RwLock::new(None),
            },
            this: weak_self.clone(),
            children: RwMutex::new(Children::new()),
        })
    }

    /// Gets the name of the `Dentry_`.
    ///
    /// Returns "/" if it is a root `Dentry_`.
    pub fn name(&self) -> String {
        match self.name_and_parent.read().as_ref() {
            Some(name_and_parent) => name_and_parent.0.clone(),
            None => String::from("/"),
        }
    }

    /// Gets the parent `Dentry_`.
    ///
    /// Returns None if it is a root `Dentry_`.
    pub fn parent(&self) -> Option<Arc<Self>> {
        self.name_and_parent
            .read()
            .as_ref()
            .map(|name_and_parent| name_and_parent.1.clone())
    }

    fn set_name_and_parent(&self, name: &str, parent: Arc<Self>) {
        let mut name_and_parent = self.name_and_parent.write();
        *name_and_parent = Some((String::from(name), parent));
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Gets the corresponding unique `DentryKey`.
    pub fn key(&self) -> DentryKey {
        DentryKey::new(self)
    }

    /// Gets the inner inode.
    pub fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    fn flags(&self) -> DentryFlags {
        let flags = self.flags.load(Ordering::Relaxed);
        DentryFlags::from_bits(flags).unwrap()
    }

    /// Checks if this dentry is a descendant (child, grandchild, or
    /// great-grandchild, etc.) of another dentry.
    pub fn is_descendant_of(&self, ancestor: &Arc<Self>) -> bool {
        let mut parent = self.parent();
        while let Some(p) = parent {
            if Arc::ptr_eq(&p, ancestor) {
                return true;
            }
            parent = p.parent();
        }
        false
    }

    pub fn is_mountpoint(&self) -> bool {
        self.flags().contains(DentryFlags::MOUNTED)
    }

    pub fn set_mountpoint_dentry(&self) {
        self.flags
            .fetch_or(DentryFlags::MOUNTED.bits(), Ordering::Release);
    }

    pub fn clear_mountpoint(&self) {
        self.flags
            .fetch_and(!(DentryFlags::MOUNTED.bits()), Ordering::Release);
    }

    /// Currently, the root `Dentry_` of a fs is the root of a mount.
    pub fn is_root_of_mount(&self) -> bool {
        self.name_and_parent.read().as_ref().is_none()
    }

    /// Creates a `Dentry_` by creating a new inode of the `type_` with the `mode`.
    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains(name) {
            return_errno!(Errno::EEXIST);
        }

        let new_inode = self.inode.create(name, type_, mode)?;
        let name = String::from(name);
        let new_child = Dentry_::new(new_inode, DentryOptions::Leaf((name.clone(), self.this())));

        let mut children = children.upgrade();
        children.insert(name, new_child.clone());
        Ok(new_child)
    }

    /// Lookups a target `Dentry_` from the cache in children.
    pub fn lookup_via_cache(&self, name: &str) -> Option<Arc<Dentry_>> {
        let children = self.children.read();
        children.find(name)
    }

    /// Lookups a target `Dentry_` from the file system.
    pub fn lookup_via_fs(&self, name: &str) -> Result<Arc<Dentry_>> {
        let children = self.children.upread();

        let inode = self.inode.lookup(name)?;
        let name = String::from(name);
        let target = Self::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        let mut children = children.upgrade();
        children.insert(name, target.clone());
        Ok(target)
    }

    /// Creates a `Dentry_` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<Self>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains(name) {
            return_errno!(Errno::EEXIST);
        }

        let inode = self.inode.mknod(name, mode, type_)?;
        let name = String::from(name);
        let new_child = Dentry_::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        let mut children = children.upgrade();
        children.insert(name, new_child.clone());
        Ok(new_child)
    }

    /// Links a new name for the `Dentry_` by `link()` the inner inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains(name) {
            return_errno!(Errno::EEXIST);
        }

        let old_inode = old.inode();
        self.inode.link(old_inode, name)?;
        let name = String::from(name);
        let dentry = Dentry_::new(
            old_inode.clone(),
            DentryOptions::Leaf((name.clone(), self.this())),
        );

        let mut children = children.upgrade();
        children.insert(name, dentry);
        Ok(())
    }

    /// Deletes a `Dentry_` by `unlink()` the inner inode.
    pub fn unlink(&self, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        children.check_mountpoint(name)?;

        self.inode.unlink(name)?;

        let mut children = children.upgrade();
        children.delete(name);
        Ok(())
    }

    /// Deletes a directory `Dentry_` by `rmdir()` the inner inode.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        children.check_mountpoint(name)?;

        self.inode.rmdir(name)?;

        let mut children = children.upgrade();
        children.delete(name);
        Ok(())
    }

    /// Renames a `Dentry_` to the new `Dentry_` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "old_name or new_name is a directory");
        }
        if self.type_() != InodeType::Dir || new_dir.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        // The two are the same dentry, we just modify the name
        if Arc::ptr_eq(&self.this(), new_dir) {
            if old_name == new_name {
                return Ok(());
            }

            let children = self.children.upread();
            let old_dentry = children.check_mountpoint_then_find(old_name)?;
            children.check_mountpoint(new_name)?;

            self.inode.rename(old_name, &self.inode, new_name)?;

            let mut children = children.upgrade();
            match old_dentry.as_ref() {
                Some(dentry) => {
                    children.delete(old_name);
                    dentry.set_name_and_parent(new_name, self.this());
                    children.insert(new_name.to_string(), dentry.clone());
                }
                None => {
                    children.delete(new_name);
                }
            }
        } else {
            // The two are different dentries
            let (mut self_children, mut new_dir_children) =
                write_lock_children_on_two_dentries(self, new_dir);
            let old_dentry = self_children.check_mountpoint_then_find(old_name)?;
            new_dir_children.check_mountpoint(new_name)?;

            self.inode.rename(old_name, &new_dir.inode, new_name)?;
            match old_dentry.as_ref() {
                Some(dentry) => {
                    self_children.delete(old_name);
                    dentry.set_name_and_parent(new_name, new_dir.this());
                    new_dir_children.insert(new_name.to_string(), dentry.clone());
                }
                None => {
                    new_dir_children.delete(new_name);
                }
            }
        }
        Ok(())
    }
}

#[inherit_methods(from = "self.inode")]
impl Dentry_ {
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
}

impl Debug for Dentry_ {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Dentry_")
            .field("inode", &self.inode)
            .field("flags", &self.flags())
            .finish()
    }
}

/// `DentryKey` is the unique identifier for the corresponding `Dentry_`.
///
/// For none-root dentries, it uses self's name and parent's pointer to form the key,
/// meanwhile, the root `Dentry_` uses "/" and self's pointer to form the key.
#[derive(Debug, Clone, Hash, PartialOrd, Ord, Eq, PartialEq)]
pub struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    /// Forms a `DentryKey` from the corresponding `Dentry_`.
    pub fn new(dentry: &Dentry_) -> Self {
        let (name, parent) = match dentry.name_and_parent.read().as_ref() {
            Some(name_and_parent) => name_and_parent.clone(),
            None => (String::from("/"), dentry.this()),
        };
        Self {
            name,
            parent_ptr: Arc::as_ptr(&parent) as usize,
        }
    }
}

bitflags! {
    struct DentryFlags: u32 {
        const MOUNTED = 1 << 0;
    }
}

enum DentryOptions {
    Root,
    Leaf((String, Arc<Dentry_>)),
}

struct Children {
    dentries: HashMap<String, Arc<Dentry_>>,
}

impl Children {
    pub fn new() -> Self {
        Self {
            dentries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.dentries.len()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.dentries.contains_key(name)
    }

    pub fn find(&self, name: &str) -> Option<Arc<Dentry_>> {
        self.dentries.get(name).cloned()
    }

    pub fn insert(&mut self, name: String, dentry: Arc<Dentry_>) {
        // Do not cache it in the children if is not cacheable.
        // When we lookup it from the parent, it will always be newly created.
        if !dentry.inode.is_dentry_cacheable() {
            return;
        }

        let _ = self.dentries.insert(name, dentry);
    }

    pub fn delete(&mut self, name: &str) -> Option<Arc<Dentry_>> {
        self.dentries.remove(name)
    }

    pub fn check_mountpoint(&self, name: &str) -> Result<()> {
        if let Some(dentry) = self.dentries.get(name) {
            if dentry.is_mountpoint() {
                return_errno_with_message!(Errno::EBUSY, "dentry is mountpint");
            }
        }
        Ok(())
    }

    pub fn check_mountpoint_then_find(&self, name: &str) -> Result<Option<Arc<Dentry_>>> {
        let dentry = if let Some(dentry) = self.dentries.get(name) {
            if dentry.is_mountpoint() {
                return_errno_with_message!(Errno::EBUSY, "dentry is mountpint");
            }
            Some(dentry.clone())
        } else {
            None
        };
        Ok(dentry)
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a Dentry_,
    other: &'a Dentry_,
) -> (
    RwMutexWriteGuard<'a, Children>,
    RwMutexWriteGuard<'a, Children>,
) {
    let this_key = this.key();
    let other_key = other.key();
    if this_key < other_key {
        let this = this.children.write();
        let other = other.children.write();
        (this, other)
    } else {
        let other = other.children.write();
        let this = this.children.write();
        (this, other)
    }
}

impl Dentry {
    /// Creates a new `Dentry` to represent the root directory of a file system.
    pub fn new_fs_root(mount_node: Arc<MountNode>) -> Self {
        let inner = mount_node.root_dentry().clone();
        Self::new(mount_node, inner)
    }

    /// Creates a new `Dentry` to represent the child directory of a file system.
    pub fn new_fs_child(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Self> {
        let new_child_dentry = self.inner.create(name, type_, mode)?;
        Ok(Self::new(self.mount_node.clone(), new_child_dentry))
    }

    fn new(mount_node: Arc<MountNode>, inner: Arc<Dentry_>) -> Self {
        Self { mount_node, inner }
    }

    /// Lookups the target `Dentry` given the `name`.
    pub fn lookup(&self, name: &str) -> Result<Self> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if !self.mode()?.is_executable() {
            return_errno!(Errno::EACCES);
        }
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let target_dentry = match name {
            "." => self.this(),
            ".." => self.effective_parent().unwrap_or_else(|| self.this()),
            name => {
                let target_inner_opt = self.inner.lookup_via_cache(name);
                match target_inner_opt {
                    Some(target_inner) => Self::new(self.mount_node.clone(), target_inner),
                    None => {
                        let target_inner = self.inner.lookup_via_fs(name)?;
                        Self::new(self.mount_node.clone(), target_inner)
                    }
                }
            }
        };

        let target_dentry = target_dentry.get_top_dentry();
        Ok(target_dentry)
    }

    /// Gets the absolute path.
    ///
    /// It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path = self.effective_name();
        let mut dir_dentry = self.this();

        while let Some(parent_dir_dentry) = dir_dentry.effective_parent() {
            path = {
                let parent_name = parent_dir_dentry.effective_name();
                if parent_name != "/" {
                    parent_name + "/" + &path
                } else {
                    parent_name + &path
                }
            };
            dir_dentry = parent_dir_dentry;
        }
        debug_assert!(path.starts_with('/'));
        path
    }

    /// Gets the effective name of the `Dentry`.
    ///
    /// If it is the root of a mount, it will go up to the mountpoint
    /// to get the name of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.inner.is_root_of_mount() {
            return self.inner.name();
        }

        let Some(parent) = self.mount_node.parent() else {
            return self.inner.name();
        };
        let Some(mountpoint) = self.mount_node.mountpoint_dentry() else {
            return self.inner.name();
        };

        let parent_inner = Self::new(
            self.mount_node.parent().unwrap().upgrade().unwrap(),
            self.mount_node.mountpoint_dentry().unwrap(),
        );
        parent_inner.effective_name()
    }

    /// Gets the effective parent of the `Dentry`.
    ///
    /// If it is the root of a mount, it will go up to the mountpoint
    /// to get the parent of the mountpoint recursively.
    fn effective_parent(&self) -> Option<Self> {
        if !self.inner.is_root_of_mount() {
            return Some(Self::new(
                self.mount_node.clone(),
                self.inner.parent().unwrap(),
            ));
        }

        let parent = self.mount_node.parent()?;
        let mountpoint = self.mount_node.mountpoint_dentry()?;

        let parent_dentry = Self::new(parent.upgrade().unwrap(), mountpoint);
        parent_dentry.effective_parent()
    }

    /// Gets the top `Dentry` of the current.
    ///
    /// Used when different file systems are mounted on the same mount point.
    ///
    /// For example, first `mount /dev/sda1 /mnt` and then `mount /dev/sda2 /mnt`.
    /// After the second mount is completed, the content of the first mount will be overridden.
    /// We need to recursively obtain the top `Dentry`.
    fn get_top_dentry(self) -> Self {
        if !self.inner.is_mountpoint() {
            return self;
        }

        match self.mount_node.get(&self) {
            Some(child_mount) => {
                let inner = child_mount.root_dentry().clone();
                Self::new(child_mount, inner).get_top_dentry()
            }
            None => self,
        }
    }

    /// Makes current `Dentry` to be a mountpoint,
    /// sets it as the mountpoint of the child mount.
    pub(super) fn set_mountpoint(&self, child_mount: Arc<MountNode>) {
        child_mount.set_mountpoint_dentry(&self.inner);
        self.inner.set_mountpoint_dentry();
    }

    /// Mounts the fs on current `Dentry` as a mountpoint.
    ///
    /// If the given mountpoint has already been mounted,
    /// its mounted child mount will be updated.
    /// The root Dentry cannot be mounted.
    ///
    /// Returns the mounted child mount.
    pub fn mount(&self, fs: Arc<dyn FileSystem>) -> Result<Arc<MountNode>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.effective_parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "can not mount on root");
        }

        let child_mount = self.mount_node.mount(fs, &self.this())?;
        self.set_mountpoint(child_mount.clone());
        Ok(child_mount)
    }

    /// Unmounts and returns the mounted child mount.
    ///
    /// Note that the root mount cannot be unmounted.
    pub fn unmount(&self) -> Result<Arc<MountNode>> {
        if !self.inner.is_root_of_mount() {
            return_errno_with_message!(Errno::EINVAL, "not mounted");
        }

        let Some(mountpoint_dentry) = self.mount_node.mountpoint_dentry() else {
            return_errno_with_message!(Errno::EINVAL, "cannot umount root mount");
        };

        let mountpoint_mount_node = self.mount_node.parent().unwrap().upgrade().unwrap();
        let mountpoint = Self::new(mountpoint_mount_node.clone(), mountpoint_dentry.clone());

        let child_mount = mountpoint_mount_node.unmount(&mountpoint)?;
        mountpoint_dentry.clear_mountpoint();
        Ok(child_mount)
    }

    /// Creates a `Dentry` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Self> {
        let inner = self.inner.mknod(name, mode, type_)?;
        Ok(Self::new(self.mount_node.clone(), inner))
    }

    /// Links a new name for the `Dentry`.
    pub fn link(&self, old: &Self, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&old.mount_node, &self.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }
        self.inner.link(&old.inner, name)
    }

    /// Deletes a `Dentry`.
    pub fn unlink(&self, name: &str) -> Result<()> {
        self.inner.unlink(name)
    }

    /// Deletes a directory `Dentry`.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        self.inner.rmdir(name)
    }

    /// Renames a `Dentry` to the new `Dentry` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Self, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.mount_node, &new_dir.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }
        self.inner.rename(old_name, &new_dir.inner, new_name)
    }

    /// Binds mount the `Dentry` to the destination `Dentry`.
    ///
    /// If `recursive` is true, it will bind mount the whole mount tree
    /// to the destination `Dentry`. Otherwise, it will only bind mount
    /// the root mount node.
    pub fn bind_mount_to(&self, dst_dentry: &Self, recursive: bool) -> Result<()> {
        let src_mount = self
            .mount_node
            .clone_mount_node_tree(&self.inner, recursive);
        src_mount.graft_mount_node_tree(dst_dentry)?;
        Ok(())
    }

    fn this(&self) -> Self {
        self.clone()
    }

    /// Gets the mount node of current `Dentry`.
    pub fn mount_node(&self) -> &Arc<MountNode> {
        &self.mount_node
    }
}

#[inherit_methods(from = "self.inner")]
impl Dentry {
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
    pub fn is_root_of_mount(&self) -> bool;
    pub fn is_mountpoint(&self) -> bool;
}
