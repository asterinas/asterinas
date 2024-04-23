// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use inherit_methods_macro::inherit_methods;

use super::{FileSystem, Inode, InodeMode, InodeType, Metadata};
use crate::{
    fs::{
        device::Device,
        utils::{MountNode, NAME_MAX},
    },
    prelude::*,
    process::{Gid, Uid},
};

lazy_static! {
    static ref DCACHE: Mutex<BTreeMap<DentryKey, Arc<Dentry>>> = Mutex::new(BTreeMap::new());
}

/// The dentry cache to accelerate path lookup
pub struct Dentry {
    inode: Arc<dyn Inode>,
    name_and_parent: RwLock<Option<(String, Arc<Dentry>)>>,
    this: Weak<Dentry>,
    children: Mutex<Children>,
    flags: AtomicU32,
}

impl Dentry {
    /// Create a new root dentry with the giving inode.
    ///
    /// It is been created during the construction of MountNode struct. The MountNode
    /// struct holds an arc reference to this root dentry.
    pub(super) fn new_root(inode: Arc<dyn Inode>) -> Arc<Self> {
        let root = Self::new(inode, DentryOptions::Root);
        DCACHE.lock().insert(root.key(), root.clone());
        root
    }

    /// Internal constructor.
    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            inode,
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            name_and_parent: match options {
                DentryOptions::Leaf(name_and_parent) => RwLock::new(Some(name_and_parent)),
                _ => RwLock::new(None),
            },
            this: weak_self.clone(),
            children: Mutex::new(Children::new()),
        })
    }

    /// Get the name of dentry.
    ///
    /// Returns "/" if it is a root dentry.
    fn name(&self) -> String {
        match self.name_and_parent.read().as_ref() {
            Some(name_and_parent) => name_and_parent.0.clone(),
            None => String::from("/"),
        }
    }

    /// Get the parent.
    ///
    /// Returns None if it is root dentry.
    fn parent(&self) -> Option<Arc<Self>> {
        self.name_and_parent
            .read()
            .as_ref()
            .map(|name_and_parent| name_and_parent.1.clone())
    }

    fn set_name_and_parent(&self, name: &str, parent: Arc<Self>) {
        let mut name_and_parent = self.name_and_parent.write();
        *name_and_parent = Some((String::from(name), parent));
    }

    /// Get the arc reference to self.
    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Get the DentryKey.
    fn key(&self) -> DentryKey {
        DentryKey::new(self)
    }

    /// Get the inode.
    pub fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    /// Get the DentryFlags.
    fn flags(&self) -> DentryFlags {
        let flags = self.flags.load(Ordering::Relaxed);
        DentryFlags::from_bits(flags).unwrap()
    }

    fn is_mountpoint(&self) -> bool {
        self.flags().contains(DentryFlags::MOUNTED)
    }

    fn set_mountpoint(&self) {
        self.flags
            .fetch_or(DentryFlags::MOUNTED.bits(), Ordering::Release);
    }

    fn clear_mountpoint(&self) {
        self.flags
            .fetch_and(!(DentryFlags::MOUNTED.bits()), Ordering::Release);
    }

    /// Currently, the root dentry of a fs is the root of a mount.
    fn is_root_of_mount(&self) -> bool {
        self.name_and_parent.read().as_ref().is_none()
    }

    /// Create a dentry by making inode.
    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let child = {
            let inode = self.inode.create(name, type_, mode)?;
            let dentry = Self::new(
                inode,
                DentryOptions::Leaf((String::from(name), self.this())),
            );
            children.insert_dentry(&dentry);
            dentry
        };
        Ok(child)
    }

    /// Lookup a dentry from DCACHE.
    fn lookup_via_cache(&self, name: &str) -> Option<Arc<Dentry>> {
        let mut children = self.children.lock();
        children.find_dentry(name)
    }

    /// Lookup a dentry from filesystem.
    fn lookuop_via_fs(&self, name: &str) -> Result<Arc<Dentry>> {
        let mut children = self.children.lock();
        let inode = self.inode.lookup(name)?;
        let dentry = Self::new(
            inode,
            DentryOptions::Leaf((String::from(name), self.this())),
        );
        children.insert_dentry(&dentry);
        Ok(dentry)
    }

    fn insert_dentry(&self, child_dentry: &Arc<Dentry>) {
        let mut children = self.children.lock();
        children.insert_dentry(child_dentry);
    }

    /// Create a dentry by making a device inode.
    fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Arc<Self>> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let child = {
            let inode = self.inode.mknod(name, mode, device)?;
            let dentry = Self::new(
                inode,
                DentryOptions::Leaf((String::from(name), self.this())),
            );
            children.insert_dentry(&dentry);
            dentry
        };
        Ok(child)
    }

    /// Link a new name for the dentry by linking inode.
    fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let old_inode = old.inode();
        self.inode.link(old_inode, name)?;
        let dentry = Self::new(
            old_inode.clone(),
            DentryOptions::Leaf((String::from(name), self.this())),
        );
        children.insert_dentry(&dentry);
        Ok(())
    }

    /// Delete a dentry by unlinking inode.
    fn unlink(&self, name: &str) -> Result<()> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        let _ = children.find_dentry_with_checking_mountpoint(name)?;
        self.inode.unlink(name)?;
        children.delete_dentry(name);
        Ok(())
    }

    /// Delete a directory dentry by rmdiring inode.
    fn rmdir(&self, name: &str) -> Result<()> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        let _ = children.find_dentry_with_checking_mountpoint(name)?;
        self.inode.rmdir(name)?;
        children.delete_dentry(name);
        Ok(())
    }

    /// Rename a dentry to the new dentry by renaming inode.
    fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "old_name or new_name is a directory");
        }
        if self.inode.type_() != InodeType::Dir || new_dir.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        // Self and new_dir are same Dentry, just modify name
        if Arc::ptr_eq(&self.this(), new_dir) {
            if old_name == new_name {
                return Ok(());
            }
            let mut children = self.children.lock();
            let old_dentry = children.find_dentry_with_checking_mountpoint(old_name)?;
            let _ = children.find_dentry_with_checking_mountpoint(new_name)?;
            self.inode.rename(old_name, &self.inode, new_name)?;
            match old_dentry.as_ref() {
                Some(dentry) => {
                    children.delete_dentry(old_name);
                    dentry.set_name_and_parent(new_name, self.this());
                    children.insert_dentry(dentry);
                }
                None => {
                    children.delete_dentry(new_name);
                }
            }
        } else {
            // Self and new_dir are different Dentry
            let (mut self_children, mut new_dir_children) =
                write_lock_children_on_two_dentries(self, new_dir);
            let old_dentry = self_children.find_dentry_with_checking_mountpoint(old_name)?;
            let _ = new_dir_children.find_dentry_with_checking_mountpoint(new_name)?;
            self.inode.rename(old_name, &new_dir.inode, new_name)?;
            match old_dentry.as_ref() {
                Some(dentry) => {
                    self_children.delete_dentry(old_name);
                    dentry.set_name_and_parent(new_name, new_dir.this());
                    new_dir_children.insert_dentry(dentry);
                }
                None => {
                    new_dir_children.delete_dentry(new_name);
                }
            }
        }
        Ok(())
    }
}

#[inherit_methods(from = "self.inode")]
impl Dentry {
    fn fs(&self) -> Arc<dyn FileSystem>;
    fn sync(&self) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn size(&self) -> usize;
    fn resize(&self, size: usize) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
}

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Dentry")
            .field("inode", &self.inode)
            .field("flags", &self.flags())
            .finish()
    }
}

/// DentryKey is the unique identifier for Dentry in DCACHE.
///
/// For none-root dentries, it uses self's name and parent's pointer to form the key,
/// meanwhile, the root dentry uses "/" and self's pointer to form the key.
#[derive(Debug, Clone, Hash, PartialOrd, Ord, Eq, PartialEq)]
pub struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    /// Form the DentryKey for the dentry.
    fn new(dentry: &Dentry) -> Self {
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
    Leaf((String, Arc<Dentry>)),
}

struct Children {
    inner: BTreeMap<String, Weak<Dentry>>,
}

impl Children {
    fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    fn insert_dentry(&mut self, dentry: &Arc<Dentry>) {
        // Do not cache it in DCACHE and children if is not cacheable.
        // When we look up it from the parent, it will always be newly created.
        if !dentry.inode().is_dentry_cacheable() {
            return;
        }

        DCACHE.lock().insert(dentry.key(), dentry.clone());
        self.inner.insert(dentry.name(), Arc::downgrade(dentry));
    }

    fn delete_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        self.inner
            .remove(name)
            .and_then(|d| d.upgrade())
            .and_then(|d| DCACHE.lock().remove(&d.key()))
    }

    fn find_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        if let Some(dentry) = self.inner.get(name) {
            dentry.upgrade().or_else(|| {
                self.inner.remove(name);
                None
            })
        } else {
            None
        }
    }

    fn find_dentry_with_checking_mountpoint(&mut self, name: &str) -> Result<Option<Arc<Dentry>>> {
        let dentry = self.find_dentry(name);
        if let Some(dentry) = dentry.as_ref() {
            if dentry.is_mountpoint() {
                return_errno_with_message!(Errno::EBUSY, "dentry is mountpint");
            }
        }
        Ok(dentry)
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a Dentry,
    other: &'a Dentry,
) -> (MutexGuard<'a, Children>, MutexGuard<'a, Children>) {
    let this_key = this.key();
    let other_key = other.key();
    if this_key < other_key {
        let this = this.children.lock();
        let other = other.children.lock();
        (this, other)
    } else {
        let other = other.children.lock();
        let this = this.children.lock();
        (this, other)
    }
}

/// The DentryMnt can represent a location in the mount tree.
#[derive(Debug)]
pub struct DentryMnt {
    mount_node: Arc<MountNode>,
    dentry: Arc<Dentry>,
    this: Weak<DentryMnt>,
}

impl DentryMnt {
    /// Create a new DentryMnt to represent the root directory of a file system.
    pub fn new_fs_root(mount_node: Arc<MountNode>) -> Arc<Self> {
        Self::new(mount_node.clone(), mount_node.root_dentry().clone())
    }

    /// Crete a new DentryMnt to represent the child directory of a file system.
    pub fn new_fs_child(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        let new_child_dentry = self.dentry.create(name, type_, mode)?;
        Ok(Self::new(self.mount_node.clone(), new_child_dentry.clone()))
    }

    /// Internal constructor.
    fn new(mount_node: Arc<MountNode>, dentry: Arc<Dentry>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            mount_node,
            dentry,
            this: weak_self.clone(),
        })
    }

    /// Lookup a dentrymnt.
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

        let dentrymnt = match name {
            "." => self.this(),
            ".." => self.effective_parent().unwrap_or_else(|| self.this()),
            name => {
                let children_dentry = self.dentry.lookup_via_cache(name);
                match children_dentry {
                    Some(dentry) => Self::new(self.mount_node().clone(), dentry.clone()),
                    None => {
                        let slow_dentry = self.dentry.lookuop_via_fs(name)?;
                        Self::new(self.mount_node().clone(), slow_dentry.clone())
                    }
                }
            }
        };
        let dentrymnt = dentrymnt.get_top_dentrymnt();
        Ok(dentrymnt)
    }

    /// Get the absolute path.
    ///
    /// It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path = self.effective_name();
        let mut dir_dentrymnt = self.this();

        while let Some(parent_dir_dentrymnt) = dir_dentrymnt.effective_parent() {
            path = {
                let parent_name = parent_dir_dentrymnt.effective_name();
                if parent_name != "/" {
                    parent_name + "/" + &path
                } else {
                    parent_name + &path
                }
            };
            dir_dentrymnt = parent_dir_dentrymnt;
        }
        debug_assert!(path.starts_with('/'));
        path
    }

    /// Get the effective name of dentrymnt.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the name
    /// of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.dentry.is_root_of_mount() {
            return self.dentry.name();
        }

        let Some(parent) = self.mount_node.parent() else {
            return self.dentry.name();
        };
        let Some(mountpoint) = self.mount_node.mountpoint_dentry() else {
            return self.dentry.name();
        };

        let parent_dentrymnt = Self::new(
            self.mount_node.parent().unwrap().upgrade().unwrap().clone(),
            self.mount_node.mountpoint_dentry().unwrap().clone(),
        );
        parent_dentrymnt.effective_name()
    }

    /// Get the effective parent of dentrymnt.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the parent
    /// of the mountpoint recursively.
    fn effective_parent(&self) -> Option<Arc<Self>> {
        if !self.dentry.is_root_of_mount() {
            return Some(Self::new(
                self.mount_node.clone(),
                self.dentry.parent().unwrap().clone(),
            ));
        }

        let parent = self.mount_node.parent()?;
        let mountpoint = self.mount_node.mountpoint_dentry()?;

        let parent_dentrymnt = Self::new(parent.upgrade().unwrap(), mountpoint.clone());
        parent_dentrymnt.effective_parent()
    }

    /// Get the top DentryMnt of self.
    ///
    /// When different file systems are mounted on the same mount point.
    /// For example, first `mount /dev/sda1 /mnt` and then `mount /dev/sda2 /mnt`.
    /// After the second mount is completed, the content of the first mount will be overridden.
    /// We need to recursively obtain the top DentryMnt.
    fn get_top_dentrymnt(&self) -> Arc<Self> {
        if !self.dentry.is_mountpoint() {
            return self.this();
        }
        match self.mount_node.get(self) {
            Some(child_mount) => Self::new(child_mount.clone(), child_mount.root_dentry().clone())
                .get_top_dentrymnt(),
            None => self.this(),
        }
    }

    /// Make this DentryMnt's dentry to be a mountpoint,
    /// and set the mountpoint of the child mount to this DentryMnt's dentry.
    fn set_mountpoint(&self, child_mount: Arc<MountNode>) {
        child_mount.set_mountpoint_dentry(self.dentry.clone());
        self.dentry.set_mountpoint();
    }

    /// Mount the fs on this DentryMnt. It will make this DentryMnt's dentry to be a mountpoint.
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
        self.set_mountpoint(child_mount.clone());
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
        let mountpoint_dentrymnt =
            Self::new(mountpoint_mount_node.clone(), mountpoint_dentry.clone());

        let child_mount = mountpoint_mount_node.umount(&mountpoint_dentrymnt)?;
        mountpoint_dentry.clear_mountpoint();
        Ok(child_mount)
    }

    /// Create a DentryMnt by making a device inode.
    pub fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Arc<Self>> {
        let dentry = self.dentry.mknod(name, mode, device)?;
        Ok(Self::new(self.mount_node.clone(), dentry.clone()))
    }

    /// Link a new name for the DentryMnt by linking inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&old.mount_node, &self.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }
        self.dentry.link(&old.dentry, name)
    }

    /// Delete a DentryMnt by unlinking inode.
    pub fn unlink(&self, name: &str) -> Result<()> {
        self.dentry.unlink(name)
    }

    /// Delete a directory dentry by rmdiring inode.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        self.dentry.rmdir(name)
    }

    /// Rename a dentry to the new dentry by renaming inode.
    pub fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.mount_node, &new_dir.mount_node) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
        }
        self.dentry.rename(old_name, &new_dir.dentry, new_name)
    }

    /// Get the arc reference to self.
    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Get the mount node of this dentrymnt.
    pub fn mount_node(&self) -> &Arc<MountNode> {
        &self.mount_node
    }
}

#[inherit_methods(from = "self.dentry")]
impl DentryMnt {
    pub fn fs(&self) -> Arc<dyn FileSystem>;
    pub fn sync(&self) -> Result<()>;
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
    pub fn key(&self) -> DentryKey;
    pub fn inode(&self) -> &Arc<dyn Inode>;
}
