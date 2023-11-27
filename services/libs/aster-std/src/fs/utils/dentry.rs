use crate::fs::device::Device;
use crate::prelude::*;

use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use super::{FileSystem, Inode, InodeMode, InodeType, Metadata, MountNode, NAME_MAX};

lazy_static! {
    static ref DCACHE: Mutex<BTreeMap<DentryKey, Arc<Dentry>>> = Mutex::new(BTreeMap::new());
}

/// The dentry cache to accelerate path lookup
pub struct Dentry {
    inode: Arc<dyn Inode>,
    name_and_parent: RwLock<Option<(String, Arc<Dentry>)>>,
    this: Weak<Dentry>,
    children: Mutex<Children>,
    mount_node: Weak<MountNode>,
    flags: AtomicU32,
}

impl Dentry {
    /// Create a new root dentry with the giving inode and mount node.
    ///
    /// It is been created during the construction of MountNode struct. The MountNode
    /// struct holds an arc reference to this root dentry, while this dentry holds a
    /// weak reference to the MountNode struct.
    pub(super) fn new_root(inode: Arc<dyn Inode>, mount: Weak<MountNode>) -> Arc<Self> {
        let root = Self::new(inode, DentryOptions::Root(mount));
        DCACHE.lock().insert(root.key(), root.clone());
        root
    }

    /// Internal constructor.
    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            inode,
            mount_node: match &options {
                DentryOptions::Root(mount) => mount.clone(),
                DentryOptions::Leaf(name_and_parent) => name_and_parent.1.mount_node.clone(),
            },
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            name_and_parent: match options {
                DentryOptions::Leaf(name_and_parent) => RwLock::new(Some(name_and_parent)),
                _ => RwLock::new(None),
            },
            this: weak_self.clone(),
            children: Mutex::new(Children::new()),
        })
    }

    /// Get the overlaid dentry of self.
    ///
    /// It will jump into the child mount if it is a mountpoint.
    fn overlaid_dentry(&self) -> Arc<Self> {
        if !self.is_mountpoint() {
            return self.this();
        }
        match self.mount_node().get(self) {
            Some(child_mount) => child_mount.root_dentry().overlaid_dentry(),
            None => self.this(),
        }
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

    /// Get the effective name of dentry.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the name
    /// of the mountpoint recursively.
    fn effective_name(&self) -> String {
        if !self.is_root_of_mount() {
            return self.name();
        }

        match self.mount_node().mountpoint_dentry() {
            Some(self_mountpoint) => self_mountpoint.effective_name(),
            None => self.name(),
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

    /// Get the effective parent of dentry.
    ///
    /// If it is the root of mount, it will go up to the mountpoint to get the parent
    /// of the mountpoint recursively.
    fn effective_parent(&self) -> Option<Arc<Self>> {
        if !self.is_root_of_mount() {
            return self.parent();
        }

        match self.mount_node().mountpoint_dentry() {
            Some(self_mountpoint) => self_mountpoint.effective_parent(),
            None => self.parent(),
        }
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
    pub fn key(&self) -> DentryKey {
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

    /// Get the mount node which the dentry belongs to.
    pub fn mount_node(&self) -> Arc<MountNode> {
        self.mount_node.upgrade().unwrap()
    }

    /// Create a dentry by making inode.
    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
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

    /// Create a dentry by making a device inode.
    pub fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Arc<Self>> {
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

    /// Lookup a dentry.
    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if !self.inode.mode().is_executable() {
            return_errno!(Errno::EACCES);
        }
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let dentry = match name {
            "." => self.this(),
            ".." => self.effective_parent().unwrap_or(self.this()),
            name => {
                let mut children = self.children.lock();
                match children.find_dentry(name) {
                    Some(dentry) => dentry.overlaid_dentry(),
                    None => {
                        let inode = self.inode.lookup(name)?;
                        let dentry = Self::new(
                            inode,
                            DentryOptions::Leaf((String::from(name), self.this())),
                        );
                        children.insert_dentry(&dentry);
                        dentry
                    }
                }
            }
        };
        Ok(dentry)
    }

    /// Link a new name for the dentry by linking inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        if !Arc::ptr_eq(&old.mount_node(), &self.mount_node()) {
            return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
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
    pub fn unlink(&self, name: &str) -> Result<()> {
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
    pub fn rmdir(&self, name: &str) -> Result<()> {
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
    pub fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
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
            if !Arc::ptr_eq(&self.mount_node(), &new_dir.mount_node()) {
                return_errno_with_message!(Errno::EXDEV, "cannot cross mount");
            }
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

    /// Mount the fs on this dentry. It will make this dentry to be a mountpoint.
    ///
    /// If the given mountpoint has already been mounted, then its mounted child mount
    /// will be updated.
    /// The root dentry cannot be mounted.
    ///
    /// Return the mounted child mount.
    pub fn mount(&self, fs: Arc<dyn FileSystem>) -> Result<Arc<MountNode>> {
        if self.inode.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.effective_parent().is_none() {
            return_errno_with_message!(Errno::EINVAL, "can not mount on root");
        }

        let child_mount = self.mount_node().mount(fs, &self.this())?;
        self.set_mountpoint();
        Ok(child_mount)
    }

    /// Unmount and return the mounted child mount.
    ///
    /// Note that the root mount cannot be unmounted.
    pub fn umount(&self) -> Result<Arc<MountNode>> {
        if !self.is_root_of_mount() {
            return_errno_with_message!(Errno::EINVAL, "not mounted");
        }

        let mount_node = self.mount_node();
        let Some(mountpoint) = mount_node.mountpoint_dentry() else {
            return_errno_with_message!(Errno::EINVAL, "cannot umount root mount");
        };

        let child_mount = mountpoint.mount_node().umount(mountpoint)?;
        mountpoint.clear_mountpoint();
        Ok(child_mount)
    }

    /// Get the filesystem the inode belongs to
    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.inode.fs()
    }

    /// Flushes all changes made to data and metadata to the device.
    pub fn sync(&self) -> Result<()> {
        self.inode.sync()
    }

    /// Get the inode metadata
    pub fn inode_metadata(&self) -> Metadata {
        self.inode.metadata()
    }

    /// Get the inode type
    pub fn inode_type(&self) -> InodeType {
        self.inode.type_()
    }

    /// Get the inode permission mode
    pub fn inode_mode(&self) -> InodeMode {
        self.inode.mode()
    }

    /// Set the inode permission mode
    pub fn set_inode_mode(&self, mode: InodeMode) {
        self.inode.set_mode(mode)
    }

    /// Get the inode length
    pub fn inode_len(&self) -> usize {
        self.inode.len()
    }

    /// Get the access timestamp
    pub fn atime(&self) -> Duration {
        self.inode.atime()
    }

    /// Set the access timestamp
    pub fn set_atime(&self, time: Duration) {
        self.inode.set_atime(time)
    }

    /// Get the modified timestamp
    pub fn mtime(&self) -> Duration {
        self.inode.mtime()
    }

    /// Set the modified timestamp
    pub fn set_mtime(&self, time: Duration) {
        self.inode.set_mtime(time)
    }

    /// Get the absolute path.
    ///
    /// It will resolve the mountpoint automatically.
    pub fn abs_path(&self) -> String {
        let mut path = self.effective_name();
        let mut dentry = self.this();

        loop {
            match dentry.effective_parent() {
                None => break,
                Some(parent_dentry) => {
                    path = {
                        let parent_name = parent_dentry.effective_name();
                        if parent_name != "/" {
                            parent_name + "/" + &path
                        } else {
                            parent_name + &path
                        }
                    };
                    dentry = parent_dentry;
                }
            }
        }

        debug_assert!(path.starts_with('/'));
        path
    }
}

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Dentry")
            .field("abs_path", &self.abs_path())
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
    pub fn new(dentry: &Dentry) -> Self {
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
    Root(Weak<MountNode>),
    Leaf((String, Arc<Dentry>)),
}

struct Children {
    inner: BTreeMap<String, Weak<Dentry>>,
}

impl Children {
    pub fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    pub fn insert_dentry(&mut self, dentry: &Arc<Dentry>) {
        // Do not cache it in DCACHE and children if is not cacheable.
        // When we look up it from the parent, it will always be newly created.
        if !dentry.inode().is_dentry_cacheable() {
            return;
        }

        DCACHE.lock().insert(dentry.key(), dentry.clone());
        self.inner.insert(dentry.name(), Arc::downgrade(dentry));
    }

    pub fn delete_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        self.inner
            .remove(name)
            .and_then(|d| d.upgrade())
            .and_then(|d| DCACHE.lock().remove(&d.key()))
    }

    pub fn find_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        if let Some(dentry) = self.inner.get(name) {
            dentry.upgrade().or_else(|| {
                self.inner.remove(name);
                None
            })
        } else {
            None
        }
    }

    pub fn find_dentry_with_checking_mountpoint(
        &mut self,
        name: &str,
    ) -> Result<Option<Arc<Dentry>>> {
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
