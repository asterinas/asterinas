// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use hashbrown::HashMap;
use ostd::sync::RwMutexWriteGuard;

use super::{is_dot, is_dot_or_dotdot, is_dotdot};
use crate::{
    fs::{
        self,
        utils::{Inode, InodeExt, InodeMode, InodeType, MknodType},
    },
    prelude::*,
};

/// A `Dentry` represents a cached filesystem node in the VFS tree.
pub(super) struct Dentry {
    inode: Arc<dyn Inode>,
    type_: InodeType,
    name_and_parent: NameAndParent,
    // FIXME: Only maintain children for directory dentries.
    children: RwMutex<DentryChildren>,
    flags: AtomicU32,
    mount_count: AtomicU32,
    this: Weak<Dentry>,
}

/// The name and parent of a `Dentry`.
enum NameAndParent {
    Real(Option<RwLock<(String, Arc<Dentry>)>>),
    Pseudo(fn(&dyn Inode) -> String),
}

/// An error returned by [`NameAndParent::set`].
#[derive(Debug)]
struct SetNameAndParentError;

impl NameAndParent {
    fn name(&self, inode: &dyn Inode) -> String {
        match self {
            NameAndParent::Real(name_and_parent) => match name_and_parent {
                Some(name_and_parent) => name_and_parent.read().0.clone(),
                None => String::from("/"),
            },
            NameAndParent::Pseudo(name_fn) => (name_fn)(inode),
        }
    }

    fn parent(&self) -> Option<Arc<Dentry>> {
        if let NameAndParent::Real(Some(name_and_parent)) = self {
            Some(name_and_parent.read().1.clone())
        } else {
            None
        }
    }

    /// Sets the name and parent of the `Dentry`.
    ///
    /// # Errors
    ///
    /// Returns `SetNameAndParentError` if the `Dentry` is a root or pseudo `Dentry`.
    fn set(
        &self,
        name: &str,
        parent: Arc<Dentry>,
    ) -> core::result::Result<(), SetNameAndParentError> {
        if let NameAndParent::Real(Some(name_and_parent)) = self {
            let mut name_and_parent = name_and_parent.write();
            *name_and_parent = (String::from(name), parent);
            Ok(())
        } else {
            Err(SetNameAndParentError)
        }
    }
}

impl Dentry {
    /// Creates a new root `Dentry` with the given inode.
    ///
    /// It is been created during the construction of the `Mount`.
    /// The `Mount` holds an arc reference to this root `Dentry`.
    pub(super) fn new_root(inode: Arc<dyn Inode>) -> Arc<Self> {
        Self::new(inode, DentryOptions::Root)
    }

    /// Creates a new pseudo `Dentry` with the given inode and name function.
    pub(super) fn new_pseudo(
        inode: Arc<dyn Inode>,
        name_fn: fn(&dyn Inode) -> String,
    ) -> Arc<Self> {
        Self::new(inode, DentryOptions::Pseudo(name_fn))
    }

    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        let name_and_parent = match options {
            DentryOptions::Root => NameAndParent::Real(None),
            DentryOptions::Leaf(name_and_parent) => {
                NameAndParent::Real(Some(RwLock::new(name_and_parent)))
            }
            DentryOptions::Pseudo(name_fn) => NameAndParent::Pseudo(name_fn),
        };

        Arc::new_cyclic(|weak_self| Self {
            type_: inode.type_(),
            inode,
            name_and_parent,
            children: RwMutex::new(DentryChildren::new()),
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            mount_count: AtomicU32::new(0),
            this: weak_self.clone(),
        })
    }

    pub(super) fn is_pseudo(&self) -> bool {
        matches!(self.name_and_parent, NameAndParent::Pseudo(_))
    }

    /// Gets the type of the `Dentry`.
    pub(super) fn type_(&self) -> InodeType {
        self.type_
    }

    /// Gets the name of the `Dentry`.
    ///
    /// Returns "/" if it is a root `Dentry`.
    pub(super) fn name(&self) -> String {
        self.name_and_parent.name(self.inode.as_ref())
    }

    /// Gets the parent `Dentry`.
    ///
    /// Returns `None` if it is a root or pseudo `Dentry`.
    pub(super) fn parent(&self) -> Option<Arc<Self>> {
        self.name_and_parent.parent()
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    /// Gets the corresponding unique `DentryKey`.
    pub(super) fn key(&self) -> DentryKey {
        DentryKey::new(self)
    }

    /// Gets the inner inode.
    pub(super) fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    /// Returns whether the dentry can be cached.
    fn is_dentry_cacheable(&self) -> bool {
        // Should we store it as a dentry flag?
        self.inode.is_dentry_cacheable()
    }

    fn flags(&self) -> DentryFlags {
        let flags = self.flags.load(Ordering::Relaxed);
        DentryFlags::from_bits(flags).unwrap()
    }

    /// Checks if this dentry is a descendant of or the same as the given
    /// ancestor dentry.
    pub(super) fn is_equal_or_descendant_of(&self, ancestor: &Arc<Self>) -> bool {
        let mut current = Some(self.this());

        while let Some(node) = current {
            if Arc::ptr_eq(&node, ancestor) {
                return true;
            }
            current = node.parent();
        }

        false
    }

    pub(super) fn is_mountpoint(&self) -> bool {
        self.flags().contains(DentryFlags::MOUNTED)
    }

    pub(super) fn inc_mount_count(&self) {
        // FIXME: Theoretically, an overflow could occur. In the future,
        // we could prevent this by implementing a global maximum mount limit.
        let old_count = self.mount_count.fetch_add(1, Ordering::Relaxed);
        if old_count == 0 {
            self.flags
                .fetch_or(DentryFlags::MOUNTED.bits(), Ordering::Relaxed);
        }
    }

    pub(super) fn dec_mount_count(&self) {
        let old_count = self.mount_count.fetch_sub(1, Ordering::Relaxed);
        if old_count == 1 {
            self.flags
                .fetch_and(!(DentryFlags::MOUNTED.bits()), Ordering::Relaxed);
        }
    }

    /// Creates a `Dentry_` by creating a new inode of the `type_` with the `mode`.
    pub(super) fn create(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<Self>> {
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
    pub(super) fn lookup_via_cache(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
        let children = self.children.read();
        children.find(name)
    }

    /// Lookups a target `Dentry` from the file system.
    pub(super) fn lookup_via_fs(&self, name: &str) -> Result<Arc<Dentry>> {
        let children = self.children.upread();

        // TODO: Add a right implementation to cache negative dentry.
        let inode = self.inode.lookup(name)?;
        let name = String::from(name);
        let target = Self::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        if target.is_dentry_cacheable() {
            children.upgrade().insert(name, target.clone());
        }

        Ok(target)
    }

    /// Creates a `Dentry` by making an inode of the `type_` with the `mode`.
    pub(super) fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<Self>> {
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
    pub(super) fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
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
            children.upgrade().insert(name.clone(), dentry.clone());
        }
        fs::notify::on_link(dentry.parent().unwrap().inode(), dentry.inode(), || name);
        Ok(())
    }

    /// Deletes a `Dentry` by `unlink()` the inner inode.
    pub(super) fn unlink(&self, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        if is_dot_or_dotdot(name) {
            return_errno_with_message!(Errno::EINVAL, "unlink on . or ..");
        }

        let children = self.children.upread();
        children.check_mountpoint(name)?;

        let mut children = children.upgrade();
        let cached_child = children.delete(name);

        let child_inode = match cached_child {
            Some(child) => {
                // Cache hit: use the cached dentry
                child.inode().clone()
            }
            None => {
                // Cache miss: need to lookup from the underlying filesystem
                drop(children);
                self.inode.lookup(name)?
            }
        };

        self.inode.unlink(name)?;

        let nlinks = child_inode.metadata().nlinks;
        fs::notify::on_link_count(&child_inode);
        if nlinks == 0 {
            // FIXME: `DELETE_SELF` should be generated after closing the last FD.
            fs::notify::on_inode_removed(&child_inode);
        }
        fs::notify::on_delete(self.inode(), &child_inode, || name.to_string());
        if nlinks == 0 {
            // Ideally, we would use `fs_event_publisher()` here to avoid creating a
            // `FsEventPublisher` instance on a dying inode. However, it isn't possible because we
            // need to disable new subscribers.
            let publisher = child_inode.fs_event_publisher_or_init();
            let removed_nr_subscribers = publisher.disable_new_and_remove_subscribers();
            child_inode
                .fs()
                .fs_event_subscriber_stats()
                .remove_subscribers(removed_nr_subscribers);
        }
        Ok(())
    }

    /// Deletes a directory `Dentry` by `rmdir()` the inner inode.
    pub(super) fn rmdir(&self, name: &str) -> Result<()> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        if is_dot(name) {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if is_dotdot(name) {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let children = self.children.upread();
        children.check_mountpoint(name)?;

        let mut children = children.upgrade();
        let cached_child = children.delete(name);

        let child_inode = match cached_child {
            Some(child) => {
                // Cache hit: use the cached dentry
                child.inode().clone()
            }
            None => {
                // Cache miss: need to lookup from the underlying filesystem
                drop(children);
                self.inode.lookup(name)?
            }
        };

        self.inode.rmdir(name)?;

        let nlinks = child_inode.metadata().nlinks;
        if nlinks == 0 {
            // FIXME: `DELETE_SELF` should be generated after closing the last FD.
            fs::notify::on_inode_removed(&child_inode);
        }
        fs::notify::on_delete(self.inode(), &child_inode, || name.to_string());
        if nlinks == 0 {
            // Ideally, we would use `fs_event_publisher()` here to avoid creating a
            // `FsEventPublisher` instance on a dying inode. However, it isn't possible because we
            // need to disable new subscribers.
            let publisher = child_inode.fs_event_publisher_or_init();
            let removed_nr_subscribers = publisher.disable_new_and_remove_subscribers();
            child_inode
                .fs()
                .fs_event_subscriber_stats()
                .remove_subscribers(removed_nr_subscribers);
        }
        Ok(())
    }

    /// Renames a `Dentry` to the new `Dentry` by `rename()` the inner inode.
    pub(super) fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
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
                    dentry.name_and_parent.set(new_name, self.this()).unwrap();
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
                    dentry
                        .name_and_parent
                        .set(new_name, new_dir.this())
                        .unwrap();
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

    /// Gets the absolute path name of this `Dentry` within the filesystem.
    pub(super) fn path_name(&self) -> String {
        let mut path_name = self.name().to_string();
        let mut current_dir = self.this();

        while let Some(parent_dir) = current_dir.parent() {
            path_name = {
                let parent_name = parent_dir.name();
                if parent_name != "/" {
                    parent_name + "/" + &path_name
                } else {
                    parent_name + &path_name
                }
            };
            current_dir = parent_dir;
        }

        debug_assert!(path_name.starts_with('/') || self.is_pseudo());
        path_name
    }
}

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Dentry")
            .field("inode", &self.inode)
            .field("type_", &self.type_)
            .field("flags", &self.flags)
            .field("mount_count", &self.mount_count)
            .finish_non_exhaustive()
    }
}

/// `DentryKey` is the unique identifier for the corresponding `Dentry`.
///
/// - For non-root dentries, it uses self's name and parent's pointer to form the key,
/// - For the root dentry, it uses "/" and self's pointer to form the key.
/// - For pseudo dentries, it uses self's name and self's pointer to form the key.
#[derive(Debug, Clone, Hash, PartialOrd, Ord, Eq, PartialEq)]
pub(super) struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    /// Forms a `DentryKey` from the corresponding `Dentry`.
    fn new(dentry: &Dentry) -> Self {
        let name = dentry.name();
        let parent = dentry.parent().unwrap_or_else(|| dentry.this());

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
    Pseudo(fn(&dyn Inode) -> String),
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
    fn new() -> Self {
        Self {
            dentries: HashMap::new(),
        }
    }

    /// Checks if a valid dentry with the given name exists.
    fn contains_valid(&self, name: &str) -> bool {
        self.dentries.get(name).is_some_and(|child| child.is_some())
    }

    /// Checks if a negative dentry with the given name exists.
    #[expect(dead_code)]
    fn contains_negative(&self, name: &str) -> bool {
        self.dentries.get(name).is_some_and(|child| child.is_none())
    }

    /// Finds a dentry by name. Returns error for negative entries.
    fn find(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
        match self.dentries.get(name) {
            Some(Some(child)) => Ok(Some(child.clone())),
            Some(None) => return_errno_with_message!(Errno::ENOENT, "found a negative dentry"),
            None => Ok(None),
        }
    }

    /// Inserts a valid cacheable dentry.
    fn insert(&mut self, name: String, dentry: Arc<Dentry>) {
        // Assume the caller has checked that the dentry is cacheable
        // and will be newly created if looked up from the parent.
        debug_assert!(dentry.is_dentry_cacheable());
        let _ = self.dentries.insert(name, Some(dentry));
    }

    /// Inserts a negative dentry.
    #[expect(dead_code)]
    fn insert_negative(&mut self, name: String) {
        let _ = self.dentries.insert(name, None);
    }

    /// Deletes a dentry by name, turning it into a negative entry if exists.
    fn delete(&mut self, name: &str) -> Option<Arc<Dentry>> {
        self.dentries.get_mut(name).and_then(Option::take)
    }

    /// Checks whether the dentry is a mount point. Returns an error if it is.
    fn check_mountpoint(&self, name: &str) -> Result<()> {
        if let Some(Some(dentry)) = self.dentries.get(name)
            && dentry.is_mountpoint()
        {
            return_errno_with_message!(Errno::EBUSY, "dentry is mountpint");
        }

        Ok(())
    }

    /// Checks if dentry is a mount point, then retrieves it.
    fn check_mountpoint_then_find(&self, name: &str) -> Result<Option<Arc<Dentry>>> {
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
