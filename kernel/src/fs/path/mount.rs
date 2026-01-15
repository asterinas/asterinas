// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use hashbrown::HashMap;
use id_alloc::IdAlloc;
use spin::Once;

use crate::{
    fs::{
        path::{
            Path,
            dentry::{Dentry, DentryKey},
            mount_namespace::MountNamespace,
        },
        utils::{FileSystem, FsFlags, InodeType},
    },
    prelude::*,
};

/// Mount propagation types.
///
/// This type defines how mount and unmount events are propagated
/// from this mount to other mounts.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub enum MountPropType {
    /// A private type is the default mount type. Mount and unmount events
    /// do not propagate to or from the private mounts.
    #[default]
    Private,
    // TODO: Implement other propagation types.
}

static ID_ALLOCATOR: Once<SpinLock<IdAlloc>> = Once::new();

/// The reserved mount ID, which represents an invalid mount.
static RESERVED_MOUNT_ID: usize = 0;

pub(super) fn init() {
    // TODO: Make it configurable.
    const MAX_MOUNT_NUM: usize = 10000;

    let mut id_allocator = IdAlloc::with_capacity(MAX_MOUNT_NUM);
    let _ = id_allocator.alloc_specific(RESERVED_MOUNT_ID).unwrap(); // Reserve mount ID 0.

    ID_ALLOCATOR.call_once(|| SpinLock::new(id_allocator));
}

bitflags! {
    pub struct PerMountFlags: u32 {
        /// Mount read-only.
        const RDONLY         = 1 << 0;
        /// Ignore suid and sgid bits.
        const NOSUID         = 1 << 1;
        /// Disallow access to device special files.
        const NODEV          = 1 << 2;
        /// Disallow program execution.
        const NOEXEC         = 1 << 3;
        /// Do not update access times.
        const NOATIME        = 1 << 10;
        /// Do not update directory access times.
        const NODIRATIME     = 1 << 11;
        /// Update atime relative to mtime/ctime.
        const RELATIME       = 1 << 21;
        /// Kernel (pseudo) mount.
        const KERNMOUNT      = 1 << 22;
        /// Always perform atime updates.
        const STRICTATIME    = 1 << 24;
    }
}

impl Default for PerMountFlags {
    fn default() -> Self {
        let empty = Self::empty();
        empty | Self::RELATIME
    }
}

impl PerMountFlags {
    /// Gets the atime policy.
    fn atime_policy(&self) -> AtimePolicy {
        if self.contains(PerMountFlags::STRICTATIME) {
            AtimePolicy::Strictatime
        } else if self.contains(PerMountFlags::NOATIME) {
            AtimePolicy::Noatime
        } else {
            AtimePolicy::Relatime
        }
    }
}

/// The policy for updating access times (atime).
///
/// A Mount can only have one of the following atime policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtimePolicy {
    Relatime,
    Noatime,
    Strictatime,
}

impl core::fmt::Display for PerMountFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.contains(PerMountFlags::RDONLY) {
            write!(f, "ro")?;
        } else {
            write!(f, "rw")?;
        };
        if self.contains(PerMountFlags::NOSUID) {
            write!(f, ",nosuid")?;
        }
        if self.contains(PerMountFlags::NODEV) {
            write!(f, ",nodev")?;
        }
        if self.contains(PerMountFlags::NOEXEC) {
            write!(f, ",noexec")?;
        }
        if self.contains(PerMountFlags::NODIRATIME) {
            write!(f, ",nodiratime")?;
        }
        let atime_policy = match self.atime_policy() {
            AtimePolicy::Relatime => "relatime",
            AtimePolicy::Noatime => "noatime",
            AtimePolicy::Strictatime => "strictatime",
        };
        write!(f, ",{}", atime_policy)
    }
}

impl From<u32> for PerMountFlags {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<PerMountFlags> for u32 {
    fn from(value: PerMountFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(PerMountFlags, {
    /// An atomic version of `PerMountFlags`.
    #[derive(Debug)]
    pub struct AtomicPerMountFlags(AtomicU32);
});

/// A `Mount` represents a mounted filesystem instance in the VFS.
///
/// Each `Mount` can be viewed as a node in the mount tree, maintaining
/// mount-related information and the structure of the mount tree.
pub struct Mount {
    /// Global unique identifier for the mount node.
    id: usize,
    /// Root dentry.
    root_dentry: Arc<Dentry>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint: RwLock<Option<Arc<Dentry>>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The parent mount node.
    parent: RwLock<Option<Weak<Mount>>>,
    /// Child mount nodes which are mounted on one dentry of self.
    pub(super) children: RwLock<HashMap<DentryKey, Arc<Self>>>,
    /// The associated mount namespace.
    mnt_ns: Weak<MountNamespace>,
    /// Propagation type of this mount (e.g., private, shared).
    propagation: RwLock<MountPropType>,
    /// The flags of this mount.
    flags: AtomicPerMountFlags,
    /// Reference to self.
    this: Weak<Self>,
}

impl Mount {
    /// Creates a root mount node with an associated FS.
    ///
    /// The root mount node is not mounted on other mount nodes (which means it has no
    /// parent). The root inode of the fs will form the inner root dentry.
    ///
    /// It is allowed to create a mount node even if the fs has been provided to another
    /// mount node. It is the fs's responsibility to ensure the data consistency.
    pub(in crate::fs) fn new_root(
        fs: Arc<dyn FileSystem>,
        mnt_ns: Weak<MountNamespace>,
    ) -> Arc<Self> {
        Self::new(fs, PerMountFlags::default(), None, mnt_ns)
    }

    /// Creates a pseudo mount node with an associated FS.
    ///
    /// This pseudo mount is not mounted on other mount nodes, has no parent, and does not
    /// belong to any mount namespace.
    pub(in crate::fs) fn new_pseudo(fs: Arc<dyn FileSystem>) -> Arc<Self> {
        Self::new(fs, PerMountFlags::KERNMOUNT, None, Weak::new())
    }

    /// The internal constructor.
    ///
    /// A root mount node has no mountpoint, while other mount nodes must have one.
    ///
    /// Here, a `Mount` is instantiated without an initial mountpoint,
    /// avoiding fixed mountpoint limitations. This allows the root mount node to
    /// exist without a mountpoint, ensuring uniformity and security, while all other
    /// mount nodes must be explicitly assigned a mountpoint to maintain structural integrity.
    fn new(
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        parent_mount: Option<Weak<Mount>>,
        mnt_ns: Weak<MountNamespace>,
    ) -> Arc<Self> {
        let id = ID_ALLOCATOR.get().unwrap().lock().alloc().unwrap();

        Arc::new_cyclic(|weak_self| Self {
            id,
            root_dentry: Dentry::new_root(fs.root_inode()),
            mountpoint: RwLock::new(None),
            parent: RwLock::new(parent_mount),
            children: RwLock::new(HashMap::new()),
            propagation: RwLock::new(MountPropType::default()),
            fs,
            mnt_ns,
            flags: AtomicPerMountFlags::new(flags),
            this: weak_self.clone(),
        })
    }

    /// Gets the mount ID.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Mounts a fs on the mountpoint, it will create a new child mount node.
    ///
    /// If the given mountpoint has already been mounted, then its mounted child mount
    /// node will be updated.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    ///
    /// It is allowed to mount a fs even if the fs has been provided to another
    /// mountpoint. It is the fs's responsibility to ensure the data consistency.
    ///
    /// Return the mounted child mount.
    pub(super) fn do_mount(
        self: &Arc<Self>,
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        mountpoint: &Arc<Dentry>,
    ) -> Result<Arc<Self>> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let key = mountpoint.key();
        let child_mount = Self::new(fs, flags, Some(Arc::downgrade(self)), self.mnt_ns.clone());
        self.children.write().insert(key, child_mount.clone());
        child_mount.set_mountpoint(mountpoint);

        Ok(child_mount)
    }

    /// Unmounts a child mount node from the mountpoint and returns it.
    ///
    /// The mountpoint should belong to this mount node, or an error is returned.
    pub(super) fn do_unmount(&self, mountpoint: &Dentry) -> Result<Arc<Self>> {
        let child_mount = self
            .children
            .write()
            .remove(&mountpoint.key())
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;

        child_mount.clear_mountpoint();

        Ok(child_mount)
    }

    /// Clones a mount node with the an root `Dentry`.
    ///
    /// The new mount node will have the same fs as the original one and
    /// have no parent and children. We should set the parent and children manually.
    ///
    /// If the `new_ns` is set, the new mount will belong to the given mount namespace.
    /// Otherwise, it will belong to the same mount namespace as the current mount.
    fn clone_mount(
        &self,
        root_dentry: &Arc<Dentry>,
        new_ns: Option<&Weak<MountNamespace>>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            id: ID_ALLOCATOR.get().unwrap().lock().alloc().unwrap(),
            root_dentry: root_dentry.clone(),
            mountpoint: RwLock::new(None),
            parent: RwLock::new(None),
            children: RwLock::new(HashMap::new()),
            propagation: RwLock::new(MountPropType::default()),
            fs: self.fs.clone(),
            mnt_ns: new_ns.cloned().unwrap_or_else(|| self.mnt_ns.clone()),
            flags: AtomicPerMountFlags::new(self.flags.load(Ordering::Relaxed)),
            this: weak_self.clone(),
        })
    }

    /// Clones a mount tree starting from the specified root `Dentry`.
    ///
    /// The new mount tree will replicate the structure of the original tree.
    /// The new tree is a separate entity rooted at the given `Dentry`,
    /// and the original tree remains unchanged.
    ///
    /// If `recursive` is set to `true`, the entire tree will be copied.
    /// Otherwise, only the root mount node will be copied.
    ///
    /// If the `new_ns` is set, the new mount tree will belong to the given mount namespace.
    /// Otherwise, it will belong to the same mount namespace as the current mount.
    pub(super) fn clone_mount_tree(
        &self,
        root_dentry: &Arc<Dentry>,
        new_ns: Option<&Weak<MountNamespace>>,
        recursive: bool,
    ) -> Arc<Self> {
        let new_root_mount = self.clone_mount(root_dentry, new_ns);
        if !recursive {
            return new_root_mount;
        }

        let mut stack = vec![self.this()];
        let mut new_stack = vec![new_root_mount.clone()];
        while let Some(old_mount) = stack.pop() {
            let new_parent_mount = new_stack.pop().unwrap();
            let old_children = old_mount.children.read();
            for old_child_mount in old_children.values() {
                let mountpoint = old_child_mount.mountpoint().unwrap();
                if !mountpoint.is_equal_or_descendant_of(new_parent_mount.root_dentry()) {
                    continue;
                }
                let new_child_mount =
                    old_child_mount.clone_mount(old_child_mount.root_dentry(), new_ns);
                let key = mountpoint.key();
                new_parent_mount
                    .children
                    .write()
                    .insert(key, new_child_mount.clone());
                new_child_mount.set_parent(Some(&new_parent_mount));
                new_child_mount.set_mountpoint(&old_child_mount.mountpoint().unwrap());
                stack.push(old_child_mount.clone());
                new_stack.push(new_child_mount);
            }
        }

        new_root_mount
    }

    /// Sets the propagation type of this mount.
    pub(super) fn set_propagation(&self, prop: MountPropType, recursive: bool) {
        *self.propagation.write() = prop;
        if !recursive {
            return;
        }

        let mut worklist: VecDeque<Arc<Mount>> = self.children.read().values().cloned().collect();
        while let Some(mount) = worklist.pop_front() {
            *mount.propagation.write() = prop;
            worklist.extend(mount.children.read().values().cloned());
        }
    }

    /// Detaches the mount node from the parent mount node.
    pub(super) fn detach_from_parent(&self) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            let child = parent
                .children
                .write()
                .remove(&self.mountpoint().unwrap().key());

            if let Some(child) = child {
                child.clear_mountpoint();
            }
        }
    }

    /// Attaches the mount node to the mountpoint.
    fn attach_to_path(&self, target_path: &Path) {
        let key = target_path.dentry.key();
        target_path
            .mount_node()
            .children
            .write()
            .insert(key, self.this());
        self.set_parent(Some(target_path.mount_node()));
        self.set_mountpoint(&target_path.dentry);
    }

    /// Grafts the mount node tree to the mountpoint.
    pub(super) fn graft_mount_tree(&self, target_path: &Path) -> Result<()> {
        self.detach_from_parent();
        self.attach_to_path(target_path);
        Ok(())
    }

    /// Gets a child mount node from the mountpoint if any.
    pub(super) fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        self.children.read().get(&mountpoint.key()).cloned()
    }

    /// Gets the root `Dentry` of this mount node.
    pub(super) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    /// Gets the mountpoint `Dentry` of this mount node if any.
    pub(super) fn mountpoint(&self) -> Option<Arc<Dentry>> {
        self.mountpoint.read().clone()
    }

    /// Sets the mountpoint.
    pub(super) fn set_mountpoint(&self, dentry: &Arc<Dentry>) {
        let mut mountpoint = self.mountpoint.write();
        if let Some(mountpoint) = mountpoint.as_deref() {
            mountpoint.dec_mount_count();
        }

        dentry.inc_mount_count();
        *mountpoint = Some(dentry.clone());
    }

    /// Clears the mountpoint.
    pub(super) fn clear_mountpoint(&self) {
        let mut mountpoint = self.mountpoint.write();
        if let Some(mountpoint) = mountpoint.as_deref() {
            mountpoint.dec_mount_count();
        }

        *mountpoint = None;
    }

    /// Flushes all pending filesystem metadata and cached file data to the device.
    pub(super) fn sync(&self) -> Result<()> {
        self.fs.sync()?;
        Ok(())
    }

    pub(super) fn remount(
        &self,
        mount_flags: PerMountFlags,
        fs_flags: Option<FsFlags>,
        data: Option<CString>,
        ctx: &Context,
    ) -> Result<()> {
        // TODO: This lock is a workaround to guarantee the atomicity of remount operation.
        // We need to re-design the lock mechanism of `Mount` and file system in the future.
        static REMOUNT_LOCK: Mutex<()> = Mutex::new(());

        let _guard = REMOUNT_LOCK.lock();

        if let Some(flags) = fs_flags {
            self.fs.set_fs_flags(flags, data, ctx)?;
        }

        // The logics here are consistent with Linux.
        // In Linux, `NOATIME`, `RELATIME`, and `STRICTATIME` are mutually exclusive.
        // If none of them nor `NODIRATIME` are set, the atime policy will be inherited
        // from the old flags.
        // Reference: https://elixir.bootlin.com/linux/v6.17/source/fs/namespace.c#L4097
        const ATIME_MASK: PerMountFlags = PerMountFlags::NOATIME
            .union(PerMountFlags::RELATIME)
            .union(PerMountFlags::STRICTATIME);

        let need_inherit_atime = !mount_flags.intersects(ATIME_MASK | PerMountFlags::NODIRATIME);

        if need_inherit_atime {
            let old_flags = self.flags.load(Ordering::Relaxed);
            let new_flags = mount_flags | (old_flags & ATIME_MASK);
            self.flags.store(new_flags, Ordering::Relaxed);
        } else {
            self.flags.store(mount_flags, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Gets the parent mount node if any.
    pub(super) fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    /// Gets the associated mount namespace.
    pub(super) fn mnt_ns(&self) -> &Weak<MountNamespace> {
        &self.mnt_ns
    }

    /// Gets the associated FS.
    pub(super) fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    pub(super) fn flags(&self) -> PerMountFlags {
        self.flags.load(Ordering::Relaxed)
    }

    /// Sets the parent mount node.
    ///
    /// In some cases we may need to reset the parent of
    /// the created Mount, such as move mount.
    pub(super) fn set_parent(&self, mount: Option<&Arc<Mount>>) {
        let mut parent = self.parent.write();
        *parent = mount.map(Arc::downgrade);
    }

    /// Finds the corresponding `Mount` in the given mount namespace.
    pub(super) fn find_corresponding_mount(
        &self,
        mnt_ns: &Arc<MountNamespace>,
    ) -> Option<Arc<Self>> {
        // Collect the ancestors from self to the root mount (The root mount is not included).
        let mut ancestors = VecDeque::new();
        let mut current = self.this();
        while let Some(weak_parent) = current.parent() {
            ancestors.push_front(current.clone());
            current = weak_parent.upgrade().unwrap();
        }

        let mut target_mount = mnt_ns.root().clone();
        while let Some(ancestor) = ancestors.pop_front() {
            // Find the child mount that matches the mountpoint of ancestor.
            let mount_point = ancestor.mountpoint().unwrap();
            let child_mount = target_mount
                .children
                .read()
                .get(&mount_point.key())
                .cloned();
            if let Some(child_mount) = child_mount {
                target_mount = child_mount;
            } else {
                return None;
            }
        }

        Some(target_mount)
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }
}

impl Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("root", &self.root_dentry)
            .field("mountpoint", &self.mountpoint)
            .field("fs", &self.fs)
            .finish()
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        ID_ALLOCATOR.get().unwrap().lock().free(self.id);
    }
}
