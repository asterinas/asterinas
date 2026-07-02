// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::Deref,
    sync::atomic::{AtomicU32, Ordering},
};

use hashbrown::HashMap;
use ostd::sync::{RwMutexUpgradeableGuard, RwMutexWriteGuard};

use super::{RenameMode, is_dot, is_dot_or_dotdot, is_dotdot};
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
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

/// A `Dentry` represents a cached filesystem node in the VFS tree.
///
/// # Dentry variants
///
/// Conceptually, there are four variants of dentries:
///
/// - **Root** — the root of a mounted filesystem;
///   its name derives from the mountpoint.
///   It has no in-tree parent.
/// - **Named** — an ordinary entry under a parent directory
///   with a real, mutable name.
/// - **Anonymous** — a real filesystem inode that has no name yet:
///   it keeps a real parent directory,
///   but is deliberately kept out of the parent's children cache,
///   so path lookup cannot reach it.
///   This variant is reserved for temporary files created via `O_TMPFILE`;
///   such a file can be promoted to a `Named` dentry
///   by a later `linkat(2)` (future work).
/// - **Pseudo** — an object with no position in any real filesystem tree
///   (pipes, sockets, anon-inode fds, namespace/pidfd files, memfd).
///   It has no parent,
///   and its name is a synthesized display string
///   used only for `/proc/<pid>/fd/<n>`.
///
/// Only `Root` and `Named` dentries are reachable by path lookup;
/// `Anonymous` and `Pseudo` are not.
///
/// To understand their connections and differences,
/// ask the following three questions (Q1 to Q3) in the diagram below:
///
/// ```text
/// +-------------------------------------------+
/// | Q1. In a *real* filesystem tree?          |
/// +-------------------------------------------+
///   |
///   |-- no -->  +-----------------------------+
///   |           |           Pseudo            |
///   |           |  pipe, socket, anon_inode,  |
///   |           |  memfd, ns/pidfd            |
///   |           +-----------------------------+
///   | yes
///   v
/// +-------------------------------------------+
/// | Q2. Has a *parent*?                       |
/// +-------------------------------------------+
///   |
///   |-- no -->  +-----------------------------+
///   |           |            Root             |
///   |           |  root of a mounted fs       |
///   |           +-----------------------------+
///   | yes
///   v
/// +-------------------------------------------+
/// | Q3. Has a *name*?                         |
/// +-------------------------------------------+
///   |
///   |-- no -->  +-----------------------------+
///   |           |          Anonymous          |
///   |           |  O_TMPFILE: a real inode    |
///   |           |  under a real directory,    |
///   |           |  not yet / never named      |
///   |           +-----------------------------+
///   | yes
///   v
/// +-----------------------------+
/// |            Named            |
/// |  ordinary file or directory |
/// +-----------------------------+
/// ```
///
/// # Dentries vs inodes
///
/// An [`Inode`] *is* the filesystem object:
/// it owns the file's data and metadata
/// (type, size, mode, owner, timestamps, link count)
/// and is implemented per filesystem (e.g. ramfs, ext2).
/// An inode has no inherent name and no location in the directory tree.
///
/// A `Dentry` is the VFS-level cache node that *names* an inode
/// and places it in the path namespace.
/// Every `Dentry` references exactly one inode (held as `Arc<dyn Inode>`),
/// but the reverse is not one-to-one:
///
/// - **One inode, many dentries.**
///   Hard links are several `Named` dentries referencing the same inode;
///   the inode's link count tracks how many.
/// - **One inode, no reachable dentry.**
///   A file unlinked while still open,
///   or an `Anonymous` (`O_TMPFILE`) inode,
///   lives on with no `Named` dentry, so path lookup cannot reach it.
/// - **Directories are one-to-one.**
///   A directory inode cannot be hard-linked,
///   so it has exactly one `Named` (or `Root`) dentry;
///   only the `Dentry` layer holds the parent/child structure
///   and the children cache that path lookup walks.
///
/// Rule of thumb:
/// operations on *content and metadata*
/// (`read`, `write`, `stat`, `chmod`)
/// act on the [`Inode`];
/// operations on *names and tree shape*
/// (lookup, `link`, `unlink`, `rename`, mounting)
/// act on the `Dentry`.
/// A [`Path`](super::Path) goes one step further
/// and pairs a `Dentry` with the `Mount` it was reached through,
/// since mounts let a single `Dentry`
/// appear at several locations in the namespace.
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
    /// A root or named `Dentry`:
    /// `None` is the root (name `"/"`, no parent);
    /// `Some` is a named child whose name and parent can change
    /// (e.g. on `rename`).
    Real(Option<RwLock<(String, Arc<Dentry>)>>),
    /// An anonymous child:
    /// a real parent directory,
    /// but a fixed `"/"` name and never present in the parent's children cache.
    Anonymous(Arc<Dentry>),
    /// A pseudo `Dentry`;
    /// the `fn` synthesizes its display name.
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
            NameAndParent::Anonymous(_) => String::from("/"),
            NameAndParent::Pseudo(name_fn) => (name_fn)(inode),
        }
    }

    fn parent(&self) -> Option<Arc<Dentry>> {
        match self {
            NameAndParent::Real(Some(name_and_parent)) => Some(name_and_parent.read().1.clone()),
            NameAndParent::Anonymous(parent) => Some(parent.clone()),
            NameAndParent::Real(None) | NameAndParent::Pseudo(_) => None,
        }
    }

    /// Sets the name and parent of the `Dentry`.
    ///
    /// # Errors
    ///
    /// Returns `SetNameAndParentError`
    /// if the `Dentry` is a root, anonymous, or pseudo `Dentry`.
    /// (Promoting an anonymous `Dentry` to a named one,
    /// e.g. for `linkat(2)` on an `O_TMPFILE` file, is future work.)
    fn set(&self, name: &str, parent: Arc<Dentry>) -> Result<(), SetNameAndParentError> {
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

    /// Creates a new anonymous `Dentry` with the given inode and parent.
    ///
    /// See the [`Dentry`] type-level documentation for what "anonymous" means.
    pub(super) fn new_anonymous(inode: Arc<dyn Inode>, parent: &DirDentry) -> Arc<Self> {
        Self::new(
            inode,
            DentryOptions::Anonymous {
                parent: parent.this(),
            },
        )
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
            DentryOptions::Named(name_and_parent) => {
                NameAndParent::Real(Some(RwLock::new(name_and_parent)))
            }
            DentryOptions::Anonymous { parent } => NameAndParent::Anonymous(parent),
            DentryOptions::Pseudo(name_fn) => NameAndParent::Pseudo(name_fn),
        };

        let type_ = inode.type_();
        let is_dir = type_ == InodeType::Dir;
        let dir_state = is_dir.then(|| DentryDirState {
            children: RwMutex::new(DentryChildren::new()),
            revalidation_policy: inode.revalidation_policy(),
        });

        Arc::new_cyclic(|weak_self| Self {
            inode,
            type_,
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
        let children = self.validate_child_absent(name)?;
        let new_inode = self.inode.create(name, type_, mode)?;
        let mut children = children.upgrade();
        let new_child = Dentry::new(
            new_inode,
            DentryOptions::Named((String::from(name), self.this())),
        );

        Ok(self.insert_positive_child(&mut children, name, new_child))
    }

    /// Validates that `name` is absent and keeps that result stable for creation.
    fn validate_child_absent<'a>(
        &'a self,
        name: &str,
    ) -> Result<RwMutexUpgradeableGuard<'a, DentryChildren>> {
        let mut children = self.children.upread();

        if let Some(cached_entry) = children.find(name) {
            if self.revalidate_cached_entry(name, &cached_entry) {
                return match cached_entry {
                    CachedDentry::Positive { .. } => {
                        return_errno_with_message!(Errno::EEXIST, "the dentry already exists")
                    }
                    CachedDentry::Negative => Ok(children),
                };
            }

            let mut children_for_update = children.upgrade();
            let _ = children_for_update.remove(name);
            children = children_for_update.downgrade();
        }

        match self.inode.lookup(name) {
            Ok(inode) => {
                let existing_child = Dentry::new(
                    inode,
                    DentryOptions::Named((String::from(name), self.this())),
                );
                let mut children_for_update = children.upgrade();
                self.insert_positive_child(&mut children_for_update, name, existing_child);
                return_errno_with_message!(Errno::EEXIST, "the dentry already exists");
            }
            Err(error) if error.error() == Errno::ENOENT => Ok(children),
            Err(error) => Err(error),
        }
    }

    /// Inserts a positive child dentry into the directory cache.
    fn insert_positive_child(
        &self,
        children: &mut DentryChildren,
        name: &str,
        child: Arc<Dentry>,
    ) -> Arc<Dentry> {
        // TODO: Use a better storage strategy to avoid extra string allocations.
        children.insert_positive(String::from(name), child.clone());
        self.revalidate_positive_children_if_needed(children);

        child
    }

    fn has_sticky_bit(&self) -> bool {
        self.inode.metadata().mode.has_sticky_bit()
    }

    fn check_sticky_bit_permission(&self, child_inode: &Arc<dyn Inode>) -> Result<()> {
        let current_thread = current_thread!();
        let Some(posix_thread) = current_thread.as_posix_thread() else {
            return Ok(());
        };

        let dir_metadata = self.inode.metadata();
        let child_metadata = child_inode.metadata();
        let fsuid = posix_thread.credentials().fsuid();
        if fsuid == dir_metadata.uid || fsuid == child_metadata.uid {
            return Ok(());
        }

        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            posix_thread,
            CapSet::FOWNER,
        ))
    }

    /// Resolves a child dentry for a rename operation.
    ///
    /// Probes the cache with revalidation and mountpoint checks, then falls
    /// back to a filesystem lookup and caches the result.
    fn resolve_child_for_rename(
        &self,
        children: &mut DentryChildren,
        name: &str,
    ) -> Result<Arc<Dentry>> {
        if let Some(cached_entry) = children.find(name) {
            if !self.revalidate_cached_entry(name, &cached_entry) {
                let _ = children.remove(name);
            } else if cached_entry.is_mountpoint() {
                return_errno_with_message!(Errno::EBUSY, "dentry is mountpoint");
            } else {
                return match cached_entry {
                    CachedDentry::Positive { dentry } => Ok(dentry),
                    CachedDentry::Negative => {
                        return_errno_with_message!(Errno::ENOENT, "found a negative dentry")
                    }
                };
            }
        }

        self.inode.lookup(name).map(|inode| {
            let dentry = Dentry::new(
                inode,
                DentryOptions::Named((String::from(name), self.this())),
            );
            self.insert_positive_child(children, name, dentry)
        })
    }

    /// Periodically revalidates cached positive children in this directory.
    //
    // TODO: This is a workaround to keep large `DentryChildren` caches from retaining stale
    // positive entries for too long when the directory asks the VFS to revalidate
    // existing children.
    fn revalidate_positive_children_if_needed(&self, children: &mut DentryChildren) {
        const POSITIVE_CHILD_REVALIDATION_INTERVAL: usize = 1024;
        const POSITIVE_CHILD_REVALIDATION_CACHE_SIZE_THRESHOLD: usize = 1024;

        if !self
            .revalidation_policy
            .contains(RevalidationPolicy::REVALIDATE_EXISTS)
        {
            return;
        }

        // Workaround: until `DentryCache` has a proper reclamation mechanism, periodically
        // scan large caches and evict positive entries that fail `revalidate_exists`.
        children.insert_count = children.insert_count.wrapping_add(1);
        if !children
            .insert_count
            .is_multiple_of(POSITIVE_CHILD_REVALIDATION_INTERVAL)
            || children.entries.len() <= POSITIVE_CHILD_REVALIDATION_CACHE_SIZE_THRESHOLD
        {
            return;
        }

        children.revalidate_positive_entries(self);
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

        let target = Dentry::new(
            inode,
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();

        Ok(self.insert_positive_child(&mut children, name, target))
    }

    /// Creates a `Dentry` by making an inode of the `type_` with the `mode`.
    pub(super) fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<Dentry>> {
        let children = self.validate_child_absent(name)?;
        let inode = self.inode.mknod(name, mode, type_)?;
        let new_child = Dentry::new(
            inode,
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();

        Ok(self.insert_positive_child(&mut children, name, new_child))
    }

    /// Links a new `Dentry` by `link()` the old inode.
    pub(super) fn link(&self, old_inode: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let children = self.validate_child_absent(name)?;
        self.inode.link(old_inode, name)?;
        let dentry = Dentry::new(
            old_inode.clone(),
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();
        self.insert_positive_child(&mut children, name, dentry.clone());
        fs::vfs::notify::on_link(dentry.parent().unwrap().inode(), dentry.inode(), || {
            name.to_string()
        });
        Ok(())
    }

    /// Deletes a `Dentry` by `unlink()` the inner inode.
    pub(super) fn unlink(&self, name: &str) -> Result<()> {
        if is_dot_or_dotdot(name) {
            return_errno_with_message!(Errno::EISDIR, "unlink on . or ..");
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

    /// Renames the `old_name` entry in this directory to the `new_name` entry
    /// in `new_dir`.
    pub(super) fn rename(
        &self,
        old_name: &str,
        new_dir: &DirDentry,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        if is_dot_or_dotdot(old_name) {
            return_errno_with_message!(Errno::EBUSY, "old_name is . or ..");
        }
        if is_dot_or_dotdot(new_name) {
            if mode == RenameMode::NoReplace {
                return_errno_with_message!(Errno::EEXIST, "new_name is . or ..");
            } else {
                return_errno_with_message!(Errno::EBUSY, "new_name is . or ..");
            }
        }

        let old_dir_inode = self.inode();
        let new_dir_inode = new_dir.inode();

        let max_namelen = old_dir_inode.fs().sb().namelen;
        if old_name.len() > max_namelen || new_name.len() > max_namelen {
            return_errno_with_message!(Errno::ENAMETOOLONG, "old_name or new_name is too long");
        }

        if core::ptr::eq(self.inner, new_dir.inner) {
            // The two are the same dentry, we just modify the name
            if old_name == new_name {
                match mode {
                    RenameMode::Replace | RenameMode::Exchange => return Ok(()),
                    RenameMode::NoReplace => {
                        return_errno_with_message!(Errno::EEXIST, "the new path already exists");
                    }
                }
            }

            let mut children = self.children.write();

            let old_dentry = self.resolve_child_for_rename(&mut children, old_name)?;
            let new_dentry = match self.resolve_child_for_rename(&mut children, new_name) {
                Ok(dentry) => Some(dentry),
                Err(e) if e.error() == Errno::ENOENT => None,
                Err(e) => return Err(e),
            };

            Self::check_rename_mode(mode, new_dentry.as_ref())?;

            if self.has_sticky_bit() {
                self.check_sticky_bit_permission(old_dentry.inode())?;
                if let Some(new_dentry) = new_dentry.as_ref() {
                    self.check_sticky_bit_permission(new_dentry.inode())?;
                }
            }

            old_dir_inode.rename(old_name, old_dir_inode, new_name, mode)?;

            match mode {
                RenameMode::Replace | RenameMode::NoReplace => {
                    children.delete(old_name);
                    old_dentry
                        .name_and_parent
                        .set(new_name, self.this())
                        .unwrap();
                    self.insert_positive_child(&mut children, new_name, old_dentry);
                }
                RenameMode::Exchange => {
                    let new_dentry = new_dentry.unwrap();
                    old_dentry
                        .name_and_parent
                        .set(new_name, self.this())
                        .unwrap();
                    new_dentry
                        .name_and_parent
                        .set(old_name, self.this())
                        .unwrap();
                    self.insert_positive_child(&mut children, new_name, old_dentry);
                    self.insert_positive_child(&mut children, old_name, new_dentry);
                }
            }
        } else {
            // The two are different dentries
            let (mut old_children, mut new_children) =
                write_lock_children_on_two_dentries(self, new_dir);

            let old_dentry = self.resolve_child_for_rename(&mut old_children, old_name)?;
            let new_dentry = match new_dir.resolve_child_for_rename(&mut new_children, new_name) {
                Ok(dentry) => Some(dentry),
                Err(e) if e.error() == Errno::ENOENT => None,
                Err(e) => return Err(e),
            };

            Self::check_rename_mode(mode, new_dentry.as_ref())?;
            Self::check_rename_cycle(mode, self, &old_dentry, new_dir, new_dentry.as_ref())?;

            if self.has_sticky_bit() {
                self.check_sticky_bit_permission(old_dentry.inode())?;
            }
            if new_dir.has_sticky_bit()
                && let Some(new_dentry) = new_dentry.as_ref()
            {
                new_dir.check_sticky_bit_permission(new_dentry.inode())?;
            }

            old_dir_inode.rename(old_name, new_dir_inode, new_name, mode)?;

            match mode {
                RenameMode::Replace | RenameMode::NoReplace => {
                    old_children.delete(old_name);
                    old_dentry
                        .name_and_parent
                        .set(new_name, new_dir.this())
                        .unwrap();
                    new_dir.insert_positive_child(&mut new_children, new_name, old_dentry);
                }
                RenameMode::Exchange => {
                    let new_dentry = new_dentry.unwrap();
                    old_dentry
                        .name_and_parent
                        .set(new_name, new_dir.this())
                        .unwrap();
                    new_dentry
                        .name_and_parent
                        .set(old_name, self.this())
                        .unwrap();
                    new_dir.insert_positive_child(&mut new_children, new_name, old_dentry);
                    self.insert_positive_child(&mut old_children, old_name, new_dentry);
                }
            }
        }
        Ok(())
    }

    fn check_rename_cycle(
        mode: RenameMode,
        old_dir: &DirDentry<'_>,
        old_dentry: &Arc<Dentry>,
        new_dir: &DirDentry<'_>,
        new_dentry: Option<&Arc<Dentry>>,
    ) -> Result<()> {
        if old_dentry.type_() == InodeType::Dir && new_dir.is_equal_or_descendant_of(old_dentry) {
            return_errno_with_message!(Errno::EINVAL, "the new path is inside the old directory");
        }

        if mode == RenameMode::Exchange
            && let Some(new_dentry) = new_dentry
            && new_dentry.type_() == InodeType::Dir
            && old_dir.is_equal_or_descendant_of(new_dentry)
        {
            return_errno_with_message!(Errno::EINVAL, "the old path is inside the new directory");
        }

        Ok(())
    }

    fn check_rename_mode(mode: RenameMode, new_dentry: Option<&Arc<Dentry>>) -> Result<()> {
        match mode {
            RenameMode::NoReplace if new_dentry.is_some() => {
                return_errno_with_message!(Errno::EEXIST, "the new path already exists");
            }
            RenameMode::Exchange if new_dentry.is_none() => {
                return_errno_with_message!(Errno::ENOENT, "the new path does not exist");
            }
            _ => Ok(()),
        }
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
/// - For named dentries, it uses self's name and parent's pointer to form the key.
/// - For the root dentry, it uses "/" and self's pointer to form the key.
/// - For pseudo dentries, it uses self's name and self's pointer to form the key.
///
/// Anonymous dentries have no meaningful `DentryKey`:
/// they all share a fixed `"/"` name under a real parent,
/// so their keys would not be unique.
/// They are never cached by a parent or used as a mountpoint,
/// so [`Dentry::key`] must not be called on them.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    /// Forms a `DentryKey` from the corresponding `Dentry`.
    fn new(dentry: &Dentry) -> Self {
        // Anonymous dentries must not be keyed:
        // their keys would not be unique,
        // and they are never cached by a parent or used as a mountpoint.
        // See the [`DentryKey`] documentation for details.
        debug_assert!(!matches!(
            dentry.name_and_parent,
            NameAndParent::Anonymous(_)
        ));

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

/// The shape of a [`Dentry`] to create.
/// See [`Dentry`] for the taxonomy.
enum DentryOptions {
    /// Root of a mounted filesystem.
    Root,
    /// A named entry under a parent directory.
    Named((String, Arc<Dentry>)),
    /// A real inode parented to a directory
    /// but kept out of the parent's children cache.
    /// Reserved for `O_TMPFILE`.
    Anonymous { parent: Arc<Dentry> },
    /// An object with no place in any real filesystem tree;
    /// the `fn` synthesizes its display name for `/proc/<pid>/fd/<n>`.
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
    insert_count: usize,
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
static NEGATIVE_ENTRY_COUNTER: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

impl DentryChildren {
    /// Creates an empty dentry cache.
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            insert_count: 0,
        }
    }

    /// Finds a cached dentry by name.
    fn find(&self, name: &str) -> Option<CachedDentry> {
        self.entries.get(name).cloned()
    }

    /// Inserts a positive dentry.
    fn insert_positive(&mut self, name: String, dentry: Arc<Dentry>) {
        let _prev = self
            .entries
            .insert(name, CachedDentry::new_positive(dentry));

        #[cfg(debug_assertions)]
        if matches!(_prev, Some(CachedDentry::Negative)) {
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

    fn revalidate_positive_entries(&mut self, dir: &DirDentry<'_>) {
        self.entries
            .retain(|name, cached_entry| match cached_entry {
                CachedDentry::Positive { dentry } => {
                    dir.inode.revalidate_exists(name, dentry.inode().as_ref())
                }
                CachedDentry::Negative => true,
            });
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
