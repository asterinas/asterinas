// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::Deref,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};

use hashbrown::HashMap;
use ostd::sync::RwMutexWriteGuard;

use super::{is_dot, is_dot_or_dotdot, is_dotdot};
use crate::{
    fs::{
        self,
        file::{InodeMode, InodeType},
        vfs::{
            inode::{Inode, MknodType, RevalidationPolicy},
            inode_ext::InodeExt,
        },
    },
    prelude::*,
};

/// A `Dentry` represents a cached filesystem node in the VFS tree.
pub(in crate::fs) struct Dentry {
    inode: Arc<dyn Inode>,
    type_: InodeType,
    name_and_parent: NameAndParent,
    dir_state: Option<DentryDirState>,
    flags: AtomicU32,
    mount_count: AtomicU32,
    this: Weak<Dentry>,
}

/// Per-directory cache state.
struct DentryDirState {
    children: RwMutex<DentryChildren>,
    revalidation_policy: RevalidationPolicy,
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

        let type_ = inode.type_();
        let is_dir = type_ == InodeType::Dir;
        let dir_state = is_dir.then(|| DentryDirState {
            children: RwMutex::new(DentryChildren::new()),
            revalidation_policy: inode.revalidation_policy(),
        });

        Arc::new_cyclic(|weak_self| Self {
            type_,
            inode,
            name_and_parent,
            dir_state,
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

    /// Gets the absolute path name of this `Dentry` within the filesystem.
    pub(in crate::fs) fn path_name(&self) -> String {
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

    pub(super) fn as_dir_dentry_or_err(&self) -> Result<DirDentry<'_>> {
        debug_assert_eq!(self.dir_state.is_some(), self.type_ == InodeType::Dir);

        let Some(dir_state) = &self.dir_state else {
            return_errno_with_message!(
                Errno::ENOTDIR,
                "the dentry is not related to a directory inode"
            );
        };

        Ok(DirDentry {
            inner: self,
            children: &dir_state.children,
            revalidation_policy: dir_state.revalidation_policy,
        })
    }
}

/// A `Dentry` wrapper that has been validated to represent a directory.
pub(super) struct DirDentry<'a> {
    inner: &'a Dentry,
    children: &'a RwMutex<DentryChildren>,
    revalidation_policy: RevalidationPolicy,
}

impl Deref for DirDentry<'_> {
    type Target = Dentry;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl DirDentry<'_> {
    /// Creates a `Dentry` by creating a new inode of the `type_` with the `mode`.
    pub(super) fn create(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<Dentry>> {
        let children = self.children.upread();
        if let Some(entry) = children.find(name)
            && entry.is_positive()
        {
            if self.revalidate_cached_entry(name, &entry) {
                return_errno_with_message!(Errno::EEXIST, "the dentry already exists");
            }

            let mut children = children.upgrade();
            let _ = children.remove(name);
            let new_inode = self.inode.create(name, type_, mode)?;

            return Ok(self.insert_created_child(&mut children, name, new_inode));
        }

        let new_inode = self.inode.create(name, type_, mode)?;
        let mut children = children.upgrade();

        Ok(self.insert_created_child(&mut children, name, new_inode))
    }

    fn insert_created_child(
        &self,
        children: &mut DentryChildren,
        name: &str,
        inode: Arc<dyn Inode>,
    ) -> Arc<Dentry> {
        let name = String::from(name);
        let new_child = Dentry::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        children.insert_positive(name, new_child.clone());

        new_child
    }

    /// Looks up `name` relative to this dentry.
    ///
    /// On a cache miss, this method instantiates the child through the file
    /// system and inserts it into the cache before returning the resulting
    /// child dentry.
    ///
    /// This method does not interpret `"."` or `".."`, does not check search
    /// permission, and does not resolve overmounted children. Callers that need
    /// full path-component semantics should use
    /// [`super::PathResolver::lookup_at_path`].
    pub(in crate::fs) fn lookup_child(&self, name: &str) -> Result<Arc<Dentry>> {
        debug_assert!(!is_dot_or_dotdot(name));

        let mut children = self.children.upread();

        // Looks up via the dentry cache.
        if let Some(cached_entry) = children.find(name) {
            if self.revalidate_cached_entry(name, &cached_entry) {
                return match cached_entry {
                    CachedDentry::Positive { dentry } => Ok(dentry),
                    CachedDentry::Negative => {
                        return_errno_with_message!(Errno::ENOENT, "found a negative dentry")
                    }
                };
            }

            let mut children_for_update = children.upgrade();
            let _ = children_for_update.remove(name);
            children = children_for_update.downgrade();
        }

        // Looks up via the file system.

        let inode = match self.inode.lookup(name) {
            Ok(inode) => inode,
            Err(error) if error.error() == Errno::ENOENT => {
                children.upgrade().insert_negative(name.to_string());
                return Err(error);
            }
            Err(error) => return Err(error),
        };

        let name = String::from(name);
        // TODO: Use a better storage strategy to avoid extra string allocations.
        let target = Dentry::new(inode, DentryOptions::Leaf((name.clone(), self.this())));
        children.upgrade().insert_positive(name, target.clone());

        Ok(target)
    }

    /// Creates a `Dentry` by making an inode of the `type_` with the `mode`.
    pub(super) fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<Dentry>> {
        let children = self.children.upread();
        if children.contains_positive(name) {
            return_errno!(Errno::EEXIST);
        }

        let inode = self.inode.mknod(name, mode, type_)?;
        let name = String::from(name);
        let new_child = Dentry::new(inode, DentryOptions::Leaf((name.clone(), self.this())));

        children.upgrade().insert_positive(name, new_child.clone());

        Ok(new_child)
    }

    /// Links a new `Dentry` by `link()` the old inode.
    pub(super) fn link(&self, old_inode: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let children = self.children.upread();
        if children.contains_positive(name) {
            return_errno!(Errno::EEXIST);
        }

        self.inode.link(old_inode, name)?;
        let name = String::from(name);
        let dentry = Dentry::new(
            old_inode.clone(),
            DentryOptions::Leaf((name.clone(), self.this())),
        );

        children
            .upgrade()
            .insert_positive(name.clone(), dentry.clone());
        fs::vfs::notify::on_link(dentry.parent().unwrap().inode(), dentry.inode(), || name);
        Ok(())
    }

    /// Deletes a `Dentry` by `unlink()` the inner inode.
    pub(super) fn unlink(&self, name: &str) -> Result<()> {
        if is_dot_or_dotdot(name) {
            return_errno_with_message!(Errno::EINVAL, "unlink on . or ..");
        }

        let dir_inode = self.inode();
        let child_inode = self.remove_child(name, |dir_inode, name| dir_inode.unlink(name))?;

        let nlinks = child_inode.metadata().nr_hard_links;
        fs::vfs::notify::on_link_count(&child_inode);
        if nlinks == 0 {
            // FIXME: `DELETE_SELF` should be generated after closing the last FD.
            fs::vfs::notify::on_inode_removed(&child_inode);
        }
        fs::vfs::notify::on_delete(dir_inode, &child_inode, || name.to_string());
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
        if is_dot(name) {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if is_dotdot(name) {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let dir_inode = self.inode();
        let child_inode = self.remove_child(name, |dir_inode, name| dir_inode.rmdir(name))?;

        let nlinks = child_inode.metadata().nr_hard_links;
        if nlinks == 0 {
            // FIXME: `DELETE_SELF` should be generated after closing the last FD.
            fs::vfs::notify::on_inode_removed(&child_inode);
        }
        fs::vfs::notify::on_delete(dir_inode, &child_inode, || name.to_string());
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

    fn remove_child(
        &self,
        name: &str,
        remove_child_fn: impl FnOnce(&dyn Inode, &str) -> Result<()>,
    ) -> Result<Arc<dyn Inode>> {
        let dir_inode = self.inode();
        let mut children = self.children.upread();
        let cached_child = match children.find(name) {
            None => None,
            Some(cached_entry) if !self.revalidate_cached_entry(name, &cached_entry) => {
                let mut children_for_update = children.upgrade();
                let _ = children_for_update.remove(name);
                children = children_for_update.downgrade();
                None
            }
            Some(cached_entry) => {
                if cached_entry.is_mountpoint() {
                    return_errno_with_message!(Errno::EBUSY, "dentry is mountpoint");
                }

                match cached_entry {
                    CachedDentry::Positive { dentry } => Some(dentry),
                    CachedDentry::Negative => {
                        return_errno_with_message!(Errno::ENOENT, "found a negative dentry")
                    }
                }
            }
        };

        let child_inode = match &cached_child {
            Some(child) => child.inode().clone(),
            None => dir_inode.lookup(name)?,
        };

        remove_child_fn(dir_inode.as_ref(), name)?;
        if cached_child.is_some() {
            children.upgrade().delete(name);
        }
        Ok(child_inode)
    }

    /// Renames a `Dentry` to the new `Dentry` by `rename()` the inner inode.
    pub(super) fn rename(
        old_dir_arc: &Arc<Dentry>,
        old_name: &str,
        new_dir_arc: &Arc<Dentry>,
        new_name: &str,
    ) -> Result<()> {
        let old_dir = old_dir_arc.as_dir_dentry_or_err()?;
        let new_dir = new_dir_arc.as_dir_dentry_or_err()?;

        if is_dot_or_dotdot(old_name) || is_dot_or_dotdot(new_name) {
            return_errno_with_message!(Errno::EISDIR, "old_name or new_name is a directory");
        }

        let old_dir_inode = old_dir.inode();
        let new_dir_inode = new_dir.inode();

        // The two are the same dentry, we just modify the name
        if Arc::ptr_eq(old_dir_arc, new_dir_arc) {
            if old_name == new_name {
                return Ok(());
            }

            let mut children = old_dir.children.write();
            children.check_mountpoint(new_name)?;
            let old_dentry = children.probe_cached_child_for_rename(&old_dir, old_name)?;

            old_dir_inode.rename(old_name, old_dir_inode, new_name)?;

            match old_dentry.as_ref() {
                Some(dentry) => {
                    children.delete(old_name);
                    dentry
                        .name_and_parent
                        .set(new_name, old_dir_arc.clone())
                        .unwrap();
                    children.insert_positive(String::from(new_name), dentry.clone());
                }
                None => {
                    children.remove(new_name);
                }
            }
        } else {
            // The two are different dentries
            let (mut self_children, mut new_dir_children) =
                write_lock_children_on_two_dentries(&old_dir, &new_dir);
            let old_dentry = self_children.probe_cached_child_for_rename(&old_dir, old_name)?;
            new_dir_children.check_mountpoint(new_name)?;

            old_dir_inode.rename(old_name, new_dir_inode, new_name)?;
            match old_dentry.as_ref() {
                Some(dentry) => {
                    self_children.delete(old_name);
                    dentry
                        .name_and_parent
                        .set(new_name, new_dir_arc.clone())
                        .unwrap();
                    new_dir_children.insert_positive(String::from(new_name), dentry.clone());
                }
                None => {
                    new_dir_children.remove(new_name);
                }
            }
        }
        Ok(())
    }

    /// Revalidates a cached entry.
    ///
    /// Returns `true` if the cached entry is still valid, or `false` if it should be invalidated.
    fn revalidate_cached_entry(&self, name: &str, cached_entry: &CachedDentry) -> bool {
        let policy = self.revalidation_policy;
        if policy.is_empty() {
            return true;
        }

        if policy.contains(RevalidationPolicy::REVALIDATE_EXISTS)
            && let CachedDentry::Positive { dentry } = cached_entry
        {
            return self.inode.revalidate_exists(name, dentry.inode().as_ref());
        }

        if policy.contains(RevalidationPolicy::REVALIDATE_ABSENT)
            && let CachedDentry::Negative = cached_entry
        {
            return self.inode.revalidate_absent(name);
        }

        true
    }
}

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let mut debug = f.debug_struct("Dentry");
        debug
            .field("inode", &self.inode)
            .field("type_", &self.type_)
            .field("flags", &self.flags)
            .field("mount_count", &self.mount_count);
        if let Some(dir_state) = &self.dir_state {
            debug
                .field("children", &*dir_state.children.read())
                .field("revalidation_policy", &dir_state.revalidation_policy);
        }
        debug.finish_non_exhaustive()
    }
}

/// `DentryKey` is the unique identifier for the corresponding `Dentry`.
///
/// - For non-root dentries, it uses self's name and parent's pointer to form the key,
/// - For the root dentry, it uses "/" and self's pointer to form the key.
/// - For pseudo dentries, it uses self's name and self's pointer to form the key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

/// Manages child dentries in the per-directory cache.
///
/// A _negative_ dentry reflects a failed filename lookup, saving potential
/// repeated and costly lookups in the future.
//
// TODO: Implement a global reclamation mechanism for `DentryCache` to avoid unbounded growth
// of cached dentries.
#[derive(Debug)]
struct DentryChildren {
    entries: HashMap<String, CachedDentry>,
}

#[derive(Clone, Debug)]
enum CachedDentry {
    Positive { dentry: Arc<Dentry> },
    Negative,
}

impl CachedDentry {
    fn new_positive(dentry: Arc<Dentry>) -> Self {
        Self::Positive { dentry }
    }

    fn new_negative() -> Self {
        Self::Negative
    }

    fn into_dentry(self) -> Option<Arc<Dentry>> {
        match self {
            Self::Positive { dentry } => Some(dentry),
            Self::Negative => None,
        }
    }

    fn is_mountpoint(&self) -> bool {
        match self {
            Self::Positive { dentry } => dentry.is_mountpoint(),
            Self::Negative => false,
        }
    }

    fn is_positive(&self) -> bool {
        matches!(self, Self::Positive { .. })
    }
}

// TODO: Address the issue of negative dentry bloating. See the reference
// https://lwn.net/Articles/894098/ for more details.
#[cfg(debug_assertions)]
const NEGATIVE_ENTRY_LIMIT: usize = 10000;
#[cfg(debug_assertions)]
static NEGATIVE_ENTRY_COUNTER: AtomicU64 = AtomicU64::new(0);

impl DentryChildren {
    /// Creates an empty dentry cache.
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Finds a cached dentry by name.
    fn find(&self, name: &str) -> Option<CachedDentry> {
        self.entries.get(name).cloned()
    }

    /// Checks if a positive dentry with the given name exists.
    fn contains_positive(&self, name: &str) -> bool {
        self.entries
            .get(name)
            .is_some_and(|child| child.is_positive())
    }

    /// Inserts a positive dentry.
    fn insert_positive(&mut self, name: String, dentry: Arc<Dentry>) {
        let prev = self
            .entries
            .insert(name, CachedDentry::new_positive(dentry));

        #[cfg(debug_assertions)]
        if matches!(prev, Some(CachedDentry::Negative)) {
            NEGATIVE_ENTRY_COUNTER.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Inserts a negative dentry.
    ///
    /// This operation should not overwrite an existing entry.
    fn insert_negative(&mut self, name: String) {
        let prev = self.entries.insert(name, CachedDentry::new_negative());
        debug_assert!(
            prev.is_none(),
            "insert_negative overwrote an existing entry",
        );

        #[cfg(debug_assertions)]
        {
            let new_count = NEGATIVE_ENTRY_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
            if new_count > NEGATIVE_ENTRY_LIMIT as u64 {
                warn!("number of negative dentries has reached {}", new_count);
            }
        }
    }

    /// Deletes a positive dentry by name, turning it into a negative entry if exists.
    fn delete(&mut self, name: &str) {
        let Some((_, exist_entry)) = self.entries.get_key_value_mut(name) else {
            return;
        };
        if exist_entry.is_positive() {
            #[cfg(debug_assertions)]
            {
                let new_count = NEGATIVE_ENTRY_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
                if new_count > NEGATIVE_ENTRY_LIMIT as u64 {
                    warn!("number of negative dentries has reached {}", new_count);
                }
            }
            *exist_entry = CachedDentry::new_negative()
        }
    }

    /// Removes a dentry by name without installing a negative cache entry.
    fn remove(&mut self, name: &str) -> Option<Arc<Dentry>> {
        let removed_entry = self.entries.remove(name);
        #[cfg(debug_assertions)]
        if matches!(removed_entry, Some(CachedDentry::Negative)) {
            NEGATIVE_ENTRY_COUNTER.fetch_sub(1, Ordering::Relaxed);
        }

        removed_entry.and_then(CachedDentry::into_dentry)
    }

    /// Checks whether the dentry is a mount point. Returns an error if it is.
    fn check_mountpoint(&self, name: &str) -> Result<()> {
        if let Some(entry) = self.entries.get(name)
            && entry.is_mountpoint()
        {
            return_errno_with_message!(Errno::EBUSY, "dentry is mountpoint");
        }

        Ok(())
    }

    /// Probes a cached child `Dentry` before a rename operation.
    ///
    /// Returns:
    /// - `Ok(Some(entry))` for a valid positive dentry,
    /// - `Ok(None)` for a cache miss or a stale entry,
    /// - `Err(ENOENT)` for a valid negative dentry,
    /// - `Err(EBUSY)` for a dentry that is a mountpoint.
    fn probe_cached_child_for_rename(
        &mut self,
        dir: &DirDentry<'_>,
        name: &str,
    ) -> Result<Option<Arc<Dentry>>> {
        let Some(cached_entry) = self.find(name) else {
            return Ok(None);
        };

        if !dir.revalidate_cached_entry(name, &cached_entry) {
            let _ = self.remove(name);
            return Ok(None);
        }

        if cached_entry.is_mountpoint() {
            return_errno_with_message!(Errno::EBUSY, "dentry is mountpoint");
        }

        match cached_entry {
            CachedDentry::Positive { dentry } => Ok(Some(dentry)),
            CachedDentry::Negative => {
                return_errno_with_message!(Errno::ENOENT, "found a negative dentry")
            }
        }
    }
}

#[cfg(debug_assertions)]
impl Drop for DentryChildren {
    fn drop(&mut self) {
        let negative_count = self
            .entries
            .values()
            .filter(|entry| matches!(entry, CachedDentry::Negative))
            .count();

        NEGATIVE_ENTRY_COUNTER.fetch_sub(negative_count as u64, Ordering::Relaxed);
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a DirDentry,
    other: &'a DirDentry,
) -> (
    RwMutexWriteGuard<'a, DentryChildren>,
    RwMutexWriteGuard<'a, DentryChildren>,
) {
    let this_key = this.key();
    let other_key = other.key();
    match this_key.cmp(&other_key) {
        core::cmp::Ordering::Less => {
            let this = this.children.write();
            let other = other.children.write();
            (this, other)
        }
        core::cmp::Ordering::Greater => {
            let other = other.children.write();
            let this = this.children.write();
            (this, other)
        }
        core::cmp::Ordering::Equal => {
            unreachable!("two distinct DirDentry's with identical DentryKey")
        }
    }
}
