// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use hashbrown::HashMap;
use inherit_methods_macro::inherit_methods;
use ostd::sync::RwMutexWriteGuard;

use super::is_dot_or_dotdot;
use crate::{
    fs::utils::{
        FileSystem, Inode, InodeMode, InodeType, Metadata, MknodType, XattrName, XattrNamespace,
        XattrSetFlags,
    },
    prelude::*,
    process::{Gid, Uid},
};

/// A `Dentry` represents a cached filesystem node in the VFS tree.
pub(super) struct Dentry {
    inode: Arc<dyn Inode>,
    type_: InodeType,
    name_and_parent: RwLock<Option<(String, Arc<Dentry>)>>,
    children: RwMutex<DentryChildren>,
    flags: AtomicU32,
    this: Weak<Dentry>,
}

impl Dentry {
    /// Creates a new root `Dentry` with the given inode.
    ///
    /// It is been created during the construction of the `MountNode`.
    /// The `MountNode` holds an arc reference to this root `Dentry`.
    pub(super) fn new_root(inode: Arc<dyn Inode>) -> Arc<Self> {
        Self::new(inode, DentryOptions::Root)
    }

    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            type_: inode.type_(),
            inode,
            name_and_parent: match options {
                DentryOptions::Leaf(name_and_parent) => RwLock::new(Some(name_and_parent)),
                _ => RwLock::new(None),
            },
            children: RwMutex::new(DentryChildren::new()),
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            this: weak_self.clone(),
        })
    }

    /// Gets the type of the `Dentry`.
    pub fn type_(&self) -> InodeType {
        self.type_
    }

    /// Gets the name of the `Dentry`.
    ///
    /// Returns "/" if it is a root `Dentry`.
    pub fn name(&self) -> String {
        match self.name_and_parent.read().as_ref() {
            Some(name_and_parent) => name_and_parent.0.clone(),
            None => String::from("/"),
        }
    }

    /// Gets the parent `Dentry`.
    ///
    /// Returns `None` if it is a root `Dentry`.
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

    pub fn set_mounted_bit(&self) {
        self.flags
            .fetch_or(DentryFlags::MOUNTED.bits(), Ordering::Release);
    }

    pub fn clear_mounted_bit(&self) {
        self.flags
            .fetch_and(!(DentryFlags::MOUNTED.bits()), Ordering::Release);
    }

    /// Currently, the root `Dentry` of a fs is the root of a mount.
    pub fn is_mount_root(&self) -> bool {
        self.name_and_parent.read().as_ref().is_none()
    }

    /// Creates a `Dentry_` by creating a new inode of the `type_` with the `mode`.
    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains_valid(name) {
            return_errno!(Errno::EEXIST);
        }

        let new_inode = self.inode.create(name, type_, mode)?;
        let name = String::from(name);
        let new_child = Dentry::new(new_inode, DentryOptions::Leaf((name.clone(), self.this())));

        if new_child.is_dentry_cacheable() {
            children.upgrade().insert(name, new_child.clone());
        }

        Ok(new_child)
    }

    /// Lookups a target `Dentry` from the cache in children.
    pub fn lookup_via_cache(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
        let children = self.children.read();
        children.find(name)
    }

    /// Lookups a target `Dentry` from the file system.
    pub fn lookup_via_fs(&self, name: &str) -> Result<Arc<Dentry>> {
        let children = self.children.upread();

        let inode = match self.inode.lookup(name) {
            Ok(inode) => inode,
            Err(e) => {
                if e.error() == Errno::ENOENT && self.is_dentry_cacheable() {
                    children.upgrade().insert_negative(String::from(name));
                }
                return Err(e);
            }
        };
        let name = String::from(name);
        let target = Self::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        if target.is_dentry_cacheable() {
            children.upgrade().insert(name, target.clone());
        }

        Ok(target)
    }

    /// Creates a `Dentry` by making an inode of the `type_` with the `mode`.
    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<Self>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains_valid(name) {
            return_errno!(Errno::EEXIST);
        }

        let inode = self.inode.mknod(name, mode, type_)?;
        let name = String::from(name);
        let new_child = Dentry::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        if new_child.is_dentry_cacheable() {
            children.upgrade().insert(name, new_child.clone());
        }

        Ok(new_child)
    }

    /// Links a new name for the `Dentry` by `link()` the inner inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let children = self.children.upread();
        if children.contains_valid(name) {
            return_errno!(Errno::EEXIST);
        }

        let old_inode = old.inode();
        self.inode.link(old_inode, name)?;
        let name = String::from(name);
        let dentry = Dentry::new(
            old_inode.clone(),
            DentryOptions::Leaf((name.clone(), self.this())),
        );

        if dentry.is_dentry_cacheable() {
            children.upgrade().insert(name, dentry.clone());
        }
        Ok(())
    }

    /// Deletes a `Dentry` by `unlink()` the inner inode.
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

    /// Deletes a directory `Dentry` by `rmdir()` the inner inode.
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

    /// Renames a `Dentry` to the new `Dentry` by `rename()` the inner inode.
    pub fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
        if is_dot_or_dotdot(old_name) || is_dot_or_dotdot(new_name) {
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
                    if dentry.is_dentry_cacheable() {
                        children.insert(String::from(new_name), dentry.clone());
                    }
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
                    if dentry.is_dentry_cacheable() {
                        new_dir_children.insert(String::from(new_name), dentry.clone());
                    }
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
impl Dentry {
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
    pub fn is_dentry_cacheable(&self) -> bool;
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

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Dentry")
            .field("inode", &self.inode)
            .field("flags", &self.flags())
            .finish()
    }
}

/// `DentryKey` is the unique identifier for the corresponding `Dentry`.
///
/// For none-root dentries, it uses self's name and parent's pointer to form the key,
/// meanwhile, the root `Dentry` uses "/" and self's pointer to form the key.
#[derive(Debug, Clone, Hash, PartialOrd, Ord, Eq, PartialEq)]
pub(super) struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    /// Forms a `DentryKey` from the corresponding `Dentry`.
    pub(super) fn new(dentry: &Dentry) -> Self {
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

/// Manages child dentries, including both valid and negative entries.
///
/// A _negative_ dentry reflects a failed filename lookup, saving potential
/// repeated and costly lookups in the future.
// TODO: Address the issue of negative dentry bloating. See the reference
// https://lwn.net/Articles/894098/ for more details.
struct DentryChildren {
    dentries: HashMap<String, Option<Arc<Dentry>>>,
}

impl DentryChildren {
    /// Creates an empty dentry cache.
    pub fn new() -> Self {
        Self {
            dentries: HashMap::new(),
        }
    }

    /// Checks if a valid dentry with the given name exists.
    pub fn contains_valid(&self, name: &str) -> bool {
        self.dentries.get(name).is_some_and(|child| child.is_some())
    }

    /// Checks if a negative dentry with the given name exists.
    pub fn contains_negative(&self, name: &str) -> bool {
        self.dentries.get(name).is_some_and(|child| child.is_none())
    }

    /// Finds a dentry by name. Returns error for negative entries.
    pub fn find(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
        match self.dentries.get(name) {
            Some(Some(child)) => Ok(Some(child.clone())),
            Some(None) => return_errno_with_message!(Errno::ENOENT, "found a negative dentry"),
            None => Ok(None),
        }
    }

    /// Inserts a valid cacheable dentry.
    pub fn insert(&mut self, name: String, dentry: Arc<Dentry>) {
        // Assume the caller has checked that the dentry is cacheable
        // and will be newly created if looked up from the parent.
        debug_assert!(dentry.is_dentry_cacheable());
        let _ = self.dentries.insert(name, Some(dentry));
    }

    /// Inserts a negative dentry.
    pub fn insert_negative(&mut self, name: String) {
        let _ = self.dentries.insert(name, None);
    }

    /// Deletes a dentry by name, turning it into a negative entry if exists.
    pub fn delete(&mut self, name: &str) -> Option<Arc<Dentry>> {
        self.dentries.get_mut(name).and_then(Option::take)
    }

    /// Checks whether the dentry is a mount point. Returns an error if it is.
    pub fn check_mountpoint(&self, name: &str) -> Result<()> {
        if let Some(Some(dentry)) = self.dentries.get(name) {
            if dentry.is_mountpoint() {
                return_errno_with_message!(Errno::EBUSY, "dentry is mountpint");
            }
        }
        Ok(())
    }

    /// Checks if dentry is a mount point, then retrieves it.
    pub fn check_mountpoint_then_find(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
        match self.dentries.get(name) {
            Some(Some(dentry)) => {
                if dentry.is_mountpoint() {
                    return_errno_with_message!(Errno::EBUSY, "dentry is mountpoint");
                }
                Ok(Some(dentry.clone()))
            }
            Some(None) => return_errno_with_message!(Errno::ENOENT, "found a negative dentry"),
            None => Ok(None),
        }
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a Dentry,
    other: &'a Dentry,
) -> (
    RwMutexWriteGuard<'a, DentryChildren>,
    RwMutexWriteGuard<'a, DentryChildren>,
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
