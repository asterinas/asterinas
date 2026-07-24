// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use hashbrown::{HashMap, HashSet};
use sparse_id_alloc::SparseIdAlloc;

use super::{
    mount_propagation::{
        MountPropType, MountPropagation, MountTopology, PeerGroup, PeerGroupId,
        PendingPropagationChanges,
    },
    try_get_mnt_ns_inode,
};
use crate::{
    fs::{
        file::InodeType,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{
                Path,
                dentry::{Dentry, DentryKey},
                mount_namespace::MountNamespace,
            },
        },
    },
    prelude::*,
};

/// A `Mount` represents a mounted filesystem instance in the VFS.
///
/// Each `Mount` can be viewed as a node in the mount tree, maintaining
/// mount-related information and the structure of the mount tree.
pub struct Mount {
    /// Pair of recyclable and unique mount identifiers; the recyclable
    /// half is returned to the pool when the `Mount` drops.
    id: MountId,
    /// Root dentry.
    root_dentry: Arc<Dentry>,
    /// Mountpoint dentry. A mount node can be mounted on one dentry of another mount node,
    /// which makes the mount being the child of the mount node.
    mountpoint: RwLock<Option<Arc<Dentry>>>,
    /// The associated FS.
    fs: Arc<dyn FileSystem>,
    /// The mount source (e.g., a device path like "/dev/vda" or a filesystem name like "proc").
    ///
    /// The source is stored in `Mount` instead of requiring each filesystem to provide it.
    /// If a filesystem does not provide a source, it falls back to the value stored in `Mount`.
    /// This behavior aligns with that of Linux. Concrete examples can be found here:
    /// <https://github.com/asterinas/asterinas/pull/2929#discussion_r2729739818>.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/mount.h#L68>
    source: Option<String>,
    /// The parent mount node.
    parent: RwLock<Option<Weak<Mount>>>,
    /// Child mount nodes which are mounted on one dentry of self.
    pub(super) children: RwLock<HashMap<DentryKey, Arc<Self>>>,
    /// The associated mount namespace.
    mnt_ns: Weak<MountNamespace>,
    /// Propagation state of this mount.
    propagation: RwLock<MountPropagation>,
    /// The flags of this mount.
    flags: AtomicPerMountFlags,
    /// Reference to self.
    this: Weak<Self>,
}

impl Mount {
    /// Visits this mount subtree in depth-first order.
    pub(super) fn try_walk_tree(
        self: &Arc<Self>,
        mut visit_fn: impl FnMut(Arc<Self>) -> Result<()>,
    ) -> Result<()> {
        let mut pending = vec![self.clone()];

        while let Some(mount) = pending.pop() {
            let children = mount.children.read();
            pending.extend(children.values().cloned());
            drop(children);

            visit_fn(mount)?;
        }

        Ok(())
    }

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
    ) -> Result<Arc<Self>> {
        let source = fs.name().to_string();
        Self::new(fs, PerMountFlags::default(), None, mnt_ns, Some(source))
    }

    /// Creates a pseudo mount node with an associated FS.
    ///
    /// This pseudo mount is not mounted on other mount nodes, has no parent, and does not
    /// belong to any mount namespace.
    pub(in crate::fs) fn new_pseudo(fs: Arc<dyn FileSystem>) -> Result<Arc<Self>> {
        Self::new(fs, PerMountFlags::KERNMOUNT, None, Weak::new(), None)
    }

    /// Creates a mount node that is not attached to the mount tree.
    //
    // FIXME: Linux creates detached mounts in an anonymous mount namespace
    // and moves them into the target namespace when they are attached. Asterinas
    // currently records the caller's namespace here because `Mount::mnt_ns` is
    // immutable. This should be changed once mount namespaces can be updated
    // during attach.
    pub fn new_detached(
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        mnt_ns: Weak<MountNamespace>,
        source: Option<String>,
    ) -> Result<Arc<Self>> {
        Self::new(fs, flags, None, mnt_ns, source)
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
        source: Option<String>,
    ) -> Result<Arc<Self>> {
        let id = MountId::alloc()
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "mount ID space exhausted"))?;

        let mount = Arc::new_cyclic(|weak_self| Self {
            id,
            root_dentry: Dentry::new_root(fs.root_inode()),
            mountpoint: RwLock::new(None),
            fs,
            source,
            parent: RwLock::new(parent_mount),
            children: RwLock::new(HashMap::new()),
            mnt_ns,
            propagation: RwLock::new(MountPropagation::default()),
            flags: AtomicPerMountFlags::new(flags),
            this: weak_self.clone(),
        });
        // `upgrade` returns `None` for pseudo mounts (no namespace) and for
        // mounts still being built by `MountNamespace::new_with_root` (the
        // namespace is a `UniqueArc` at that point). The latter are
        // registered by `new_with_root` after the namespace is finalized.
        if let Some(ns) = mount.mnt_ns.upgrade() {
            ns.register_mount(&mount);
        }
        Ok(mount)
    }

    /// Gets the recyclable 32-bit mount ID.
    pub fn id(&self) -> RecyclableMountId {
        self.id.recyclable_id()
    }

    /// Gets the unique 64-bit mount ID.
    pub fn unique_id(&self) -> UniqueMountId {
        self.id.unique_id()
    }

    /// Returns the mount source.
    pub(in crate::fs) fn source(&self) -> Option<&str> {
        self.fs.source().or(self.source.as_deref())
    }

    /// Mounts a file system at `mountpoint` and returns the new child mount.
    ///
    /// The same file system may be mounted at multiple locations.
    /// The file system remains responsible for keeping its data consistent.
    ///
    /// A user-provided source is retained by the new mount.
    /// If this mount is shared,
    /// the attachment is also created below its propagation receivers.
    ///
    /// The caller must hold the mount-topology write lock in `topology`.
    pub(super) fn do_mount(
        self: &Arc<Self>,
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        mountpoint: &Arc<Dentry>,
        source: Option<String>,
        topology: &mut MountTopology,
    ) -> Result<Arc<Self>> {
        if mountpoint.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let child_mount = Self::new_detached(fs, flags, self.mnt_ns.clone(), source)?;
        let target_path = Path::new(self.clone(), mountpoint.clone());
        let pending_changes = PendingPropagationChanges::default();
        child_mount.attach_mount_tree_with_propagation(target_path, pending_changes, topology)?;

        Ok(child_mount)
    }

    /// Unmounts the child at `mountpoint` and returns the requested child mount.
    ///
    /// If this mount is shared,
    /// corresponding children below its propagation receivers are also unmounted.
    ///
    /// The operation is atomic: an error leaves the mount tree and propagation topology
    /// unchanged.
    /// The caller must hold the mount-topology write lock in `topology`.
    pub(super) fn do_unmount(
        self: &Arc<Self>,
        mountpoint: &Arc<Dentry>,
        topology: &mut MountTopology,
    ) -> Result<Arc<Self>> {
        let child_mount = self
            .get(mountpoint)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "can not find child mount"))?;
        let parent = child_mount
            .parent()
            .and_then(|parent| parent.upgrade())
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "the root mount cannot be unmounted")
            })?;

        let mut unmounted_mounts = vec![child_mount];
        for receiver in topology.propagation_receivers(&parent, mountpoint) {
            if let Some(child) = receiver.get(mountpoint) {
                unmounted_mounts.push(child);
            }
        }

        for target in &unmounted_mounts {
            target
                .try_walk_tree(|mount| {
                    mount.clear_mount_propagation(topology);
                    Ok(())
                })
                .unwrap();
            target.detach_from_parent(topology);
        }

        Ok(unmounted_mounts.into_iter().next().unwrap())
    }

    /// Clones this mount as an unpublished mount rooted at `root_dentry`.
    ///
    /// The clone belongs to `new_ns` and shares this mount's file system,
    /// source, and per-mount flags.
    /// It has no parent, children, mountpoint, or published propagation links.
    fn clone_mount(
        &self,
        root_dentry: &Arc<Dentry>,
        propagation: MountPropagation,
        new_ns: &Weak<MountNamespace>,
    ) -> Result<Arc<Self>> {
        let id = MountId::alloc()
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "mount ID space exhausted"))?;

        let mount = Arc::new_cyclic(|weak_self| Self {
            id,
            root_dentry: root_dentry.clone(),
            mountpoint: RwLock::new(None),
            fs: self.fs.clone(),
            source: self.source.clone(),
            parent: RwLock::new(None),
            children: RwLock::new(HashMap::new()),
            mnt_ns: new_ns.clone(),
            propagation: RwLock::new(propagation),
            flags: AtomicPerMountFlags::new(self.flags.load(Ordering::Relaxed)),
            this: weak_self.clone(),
        });
        if let Some(ns) = mount.mnt_ns.upgrade() {
            ns.register_mount(&mount);
        }
        Ok(mount)
    }

    /// Clones this mount tree and defers propagation updates.
    ///
    /// The returned tree belongs to `new_ns` and is rooted at `root_dentry`.
    /// A non-recursive clone contains only this mount;
    /// a recursive clone reproduces the eligible descendants selected by `clone_mode`.
    ///
    /// [`MountTreeCloneMode::Bind`] preserves propagation state,
    /// rejects an unbindable root,
    /// and omits unbindable child subtrees.
    /// [`MountTreeCloneMode::Namespace`] preserves propagation state
    /// and unbindable subtrees,
    /// but omits mounts rooted at mount-namespace files.
    /// [`MountTreeCloneMode::SharedPropagation`] makes corresponding source
    /// and clone mounts peers.
    /// [`MountTreeCloneMode::SlavePropagation`] makes each clone
    /// a slave of its source mount.
    ///
    /// The caller must retain `pending_changes` until every fallible step of the enclosing
    /// operation succeeds, then either commit it directly or transfer it to
    /// [`Self::attach_mount_tree_with_propagation`].
    pub(super) fn clone_mount_tree_deferred(
        self: &Arc<Self>,
        root_dentry: &Arc<Dentry>,
        new_ns: &Weak<MountNamespace>,
        recursive: bool,
        clone_mode: MountTreeCloneMode,
        pending_changes: &mut PendingPropagationChanges,
        _topology: &MountTopology,
    ) -> Result<Arc<Self>> {
        let skip_unbindable = clone_mode == MountTreeCloneMode::Bind;
        if skip_unbindable && self.is_unbindable() {
            return_errno_with_message!(Errno::EINVAL, "the source mount is unbindable");
        }

        let mut clone_mount_fn = |source_mount: &Mount,
                                  cloned_root_dentry: &Arc<Dentry>|
         -> Result<Arc<Mount>> {
            let cloned_propagation = match clone_mode {
                MountTreeCloneMode::SharedPropagation => {
                    pending_changes.make_shared_for_clone(source_mount)?
                }
                MountTreeCloneMode::SlavePropagation => {
                    let source_propagation = pending_changes.make_shared_for_clone(source_mount)?;
                    let source_peer_group_id = source_propagation
                        .peer_group_id()
                        .expect("the propagation source must be shared");
                    MountPropagation::slave(source_peer_group_id)
                }
                MountTreeCloneMode::Bind | MountTreeCloneMode::Namespace => {
                    source_mount.propagation()
                }
            };

            let cloned_mount =
                source_mount.clone_mount(cloned_root_dentry, cloned_propagation, new_ns)?;
            pending_changes.add_register_op(cloned_mount.clone());
            Ok(cloned_mount)
        };

        let new_root_mount = clone_mount_fn(self, root_dentry)?;
        let mut cloned_mounts = vec![(self.this(), new_root_mount.clone())];
        if recursive {
            let mut work_idx = 0;
            while work_idx < cloned_mounts.len() {
                let (old_parent_mount, new_parent_mount) = cloned_mounts[work_idx].clone();
                let old_children = old_parent_mount
                    .children
                    .read()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                for old_child_mount in old_children {
                    if skip_unbindable && old_child_mount.is_unbindable() {
                        continue;
                    }
                    if clone_mode == MountTreeCloneMode::Namespace
                        && try_get_mnt_ns_inode(old_child_mount.root_dentry()).is_some()
                    {
                        continue;
                    }

                    let mountpoint = old_child_mount.mountpoint().unwrap();
                    if !mountpoint.is_equal_or_descendant_of(new_parent_mount.root_dentry()) {
                        continue;
                    }

                    let new_child_mount =
                        clone_mount_fn(&old_child_mount, old_child_mount.root_dentry())?;

                    let key = mountpoint.key();
                    new_parent_mount
                        .children
                        .write()
                        .insert(key, new_child_mount.clone());
                    new_child_mount.set_parent(Some(&new_parent_mount));
                    new_child_mount.set_mountpoint(mountpoint);
                    cloned_mounts.push((old_child_mount, new_child_mount));
                }
                work_idx += 1;
            }
        }

        Ok(new_root_mount)
    }

    /// Queues a propagation-policy change for this mount.
    ///
    /// If `recursive` is `true`,
    /// the requested transition is queued independently for every mount in this subtree.
    /// The changes become visible only when `pending_changes` is committed, either directly
    /// or by transferring it to [`Self::attach_mount_tree_with_propagation`].
    pub(super) fn set_propagation_deferred(
        self: &Arc<Self>,
        prop: MountPropType,
        recursive: bool,
        pending_changes: &mut PendingPropagationChanges,
        _topology: &MountTopology,
    ) -> Result<()> {
        let mut add_set_operation_fn = |mount: Arc<Mount>| -> Result<()> {
            let new_propagation = match prop {
                MountPropType::Shared => {
                    let old_propagation = mount.propagation();
                    if old_propagation.is_shared() {
                        old_propagation
                    } else {
                        let peer_group = PeerGroup::new()?;
                        let new_propagation = old_propagation.make_shared(peer_group.id);
                        pending_changes.add_peer_group(mount.unique_id(), peer_group);
                        new_propagation
                    }
                }
                MountPropType::Slave => {
                    pending_changes.add_make_slave_op(mount);
                    return Ok(());
                }
                MountPropType::Private => MountPropagation::private(),
                MountPropType::Unbindable => MountPropagation::unbindable(),
            };
            pending_changes.add_set_op(mount, new_propagation);
            Ok(())
        };
        if recursive {
            self.try_walk_tree(add_set_operation_fn)?;
        } else {
            add_set_operation_fn(self.this())?;
        }

        Ok(())
    }

    /// Returns the space-prefixed propagation fields for `/proc/PID/mountinfo`.
    pub(in crate::fs) fn propagation_mountinfo_fields(&self) -> String {
        let propagation = self.propagation.read();
        let peer_group_id = propagation.peer_group_id();
        let master_id = propagation.master_id();
        match (peer_group_id, master_id) {
            (Some(peer_group_id), Some(master_id)) => {
                alloc::format!(" shared:{} master:{}", peer_group_id, master_id)
            }
            (Some(peer_group_id), None) => alloc::format!(" shared:{}", peer_group_id),
            (None, Some(master_id)) => alloc::format!(" master:{}", master_id),
            (None, None) => String::new(),
        }
    }

    /// Returns whether this mount cannot be bind-cloned.
    pub(super) fn is_unbindable(&self) -> bool {
        self.propagation.read().is_unbindable()
    }

    /// Returns whether this mount belongs to a peer group.
    pub(super) fn is_shared(&self) -> bool {
        self.propagation.read().is_shared()
    }

    /// Returns a snapshot of this mount's propagation state.
    pub(super) fn propagation(&self) -> MountPropagation {
        *self.propagation.read()
    }

    /// Changes the propagation master without changing other propagation state.
    pub(super) fn set_propagation_master(&self, master_id: Option<PeerGroupId>) {
        self.propagation.write().set_master_id(master_id);
    }

    /// Replaces this mount's propagation state and updates the topology indexes.
    ///
    /// The referenced peer and master groups must already be published.
    /// The caller must hold the mount-topology write lock in `topology`.
    pub(super) fn set_mount_propagation(
        self: &Arc<Self>,
        propagation: MountPropagation,
        topology: &mut MountTopology,
    ) {
        let mut propagation_guard = self.propagation.write();
        let old_propagation = *propagation_guard;
        *propagation_guard = propagation;
        drop(propagation_guard);

        let old_master_id = old_propagation.master_id();
        let new_master_id = propagation.master_id();
        if old_master_id != new_master_id {
            if let Some(old_master_id) = old_master_id {
                let group = topology
                    .peer_groups
                    .get_mut(&old_master_id)
                    .expect("master peer group must exist");
                group.slaves.remove(&self.unique_id());
            }

            if let Some(new_master_id) = new_master_id {
                let group = topology
                    .peer_groups
                    .get_mut(&new_master_id)
                    .expect("master peer group must exist");
                let _ = group.slaves.insert(self.unique_id(), Arc::downgrade(self));
            }
        }

        let old_peer_group_id = old_propagation.peer_group_id();
        let new_peer_group_id = propagation.peer_group_id();
        if old_peer_group_id != new_peer_group_id {
            if old_peer_group_id.is_some() {
                topology.remove_peer(self.unique_id(), &old_propagation);
            }
            if let Some(new_peer_group_id) = new_peer_group_id {
                let group = topology
                    .peer_groups
                    .get_mut(&new_peer_group_id)
                    .expect("peer group must exist");

                let _ = group.peers.insert(self.unique_id(), Arc::downgrade(self));
            }
        }
    }

    /// Removes this mount from propagation topology and makes it private.
    pub(super) fn clear_mount_propagation(&self, topology: &mut MountTopology) {
        let mut old_propagation = self.propagation.write();

        if let Some(master_id) = old_propagation.master_id()
            && let Some(group) = topology.peer_groups.get_mut(&master_id)
        {
            group.slaves.remove(&self.unique_id());
        }
        if old_propagation.is_shared() {
            topology.remove_peer(self.unique_id(), &old_propagation);
        }

        *old_propagation = MountPropagation::default();
    }

    /// Detaches the mount node from the parent mount node.
    pub(super) fn detach_from_parent(&self, topology: &mut MountTopology) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            let child = parent
                .children
                .write()
                .remove(&self.mountpoint().unwrap().key());

            if let Some(child) = child {
                child.clear_topology_link(topology);
            }
        }
    }

    /// Clears this mount node's topology link.
    ///
    /// The parent pointer and mountpoint describe the same topology edge, so
    /// they must be cleared together while holding the mount topology lock.
    ///
    /// This only mutates this mount node's own link state.
    pub(super) fn clear_topology_link(&self, _topology: &mut MountTopology) {
        self.set_parent(None);
        self.clear_mountpoint();
    }

    /// Attaches the mount node to the mountpoint without propagation.
    fn attach_to_path(&self, target_path: Path, _topology: &mut MountTopology) {
        let key = target_path.dentry.key();
        target_path
            .mount_node()
            .children
            .write()
            .insert(key, self.this());
        self.set_parent(Some(target_path.mount_node()));
        self.set_mountpoint(target_path.dentry);
    }

    /// Grafts the mount node tree to the mountpoint without propagation.
    pub(super) fn graft_mount_tree(&self, target_path: Path, topology: &mut MountTopology) {
        self.detach_from_parent(topology);
        self.attach_to_path(target_path, topology);
    }

    /// Attaches this mount tree and propagates the attachment.
    ///
    /// If the destination parent is shared,
    /// the tree is also cloned below every reachable peer and slave receiver.
    /// A receiver participates only if the target dentry is at or below its root dentry;
    /// a bind mount rooted elsewhere cannot address the event location.
    /// Corresponding mounts below peer receivers join the same peer groups;
    /// copies below slave receivers become slaves of the corresponding source mounts.
    ///
    /// This method takes ownership of any propagation-topology changes already prepared by
    /// the enclosing operation. After all mount-tree links are attached successfully, it
    /// publishes those changes together with the changes prepared during propagation.
    pub(super) fn attach_mount_tree_with_propagation(
        self: &Arc<Self>,
        target_path: Path,
        mut pending_changes: PendingPropagationChanges,
        topology: &mut MountTopology,
    ) -> Result<()> {
        let mut copies = Vec::new();
        let mut propagation_layers = Vec::new();
        if let Some(peer_group_id) = target_path.mount_node().propagation().peer_group_id() {
            propagation_layers.push((
                target_path.mount_node().clone(),
                self.clone(),
                peer_group_id,
            ));
        }

        while let Some((source_parent, source_mount, peer_group_id)) = propagation_layers.pop() {
            let copies_before_layer = copies.len();

            for receiver in topology.group_peers_of(peer_group_id) {
                if receiver.unique_id() == source_parent.unique_id()
                    || !target_path
                        .dentry()
                        .is_equal_or_descendant_of(receiver.root_dentry())
                {
                    continue;
                }
                let copy = source_mount.clone_mount_tree_deferred(
                    source_mount.root_dentry(),
                    receiver.mnt_ns(),
                    true,
                    MountTreeCloneMode::SharedPropagation,
                    &mut pending_changes,
                    topology,
                )?;
                copies.push((receiver, copy));
            }

            let mut seen_slave_peer_groups = HashSet::new();
            for receiver in topology.slave_mounts_of(peer_group_id) {
                if !target_path
                    .dentry()
                    .is_equal_or_descendant_of(receiver.root_dentry())
                {
                    continue;
                }
                let receiver_peer_group_id = receiver.propagation().peer_group_id();
                if receiver_peer_group_id
                    .is_some_and(|peer_group_id| !seen_slave_peer_groups.insert(peer_group_id))
                {
                    continue;
                }

                let copy = source_mount.clone_mount_tree_deferred(
                    source_mount.root_dentry(),
                    receiver.mnt_ns(),
                    true,
                    MountTreeCloneMode::SlavePropagation,
                    &mut pending_changes,
                    topology,
                )?;
                if let Some(peer_group_id) = receiver_peer_group_id {
                    propagation_layers.push((receiver.clone(), copy.clone(), peer_group_id));
                }
                copies.push((receiver, copy));
            }

            if copies.len() == copies_before_layer {
                source_mount.set_propagation_deferred(
                    MountPropType::Shared,
                    true,
                    &mut pending_changes,
                    topology,
                )?;
            }
        }

        for (receiver, copy) in copies {
            let mount_point = target_path.dentry.clone();
            debug_assert!(Arc::ptr_eq(&receiver.fs, &mount_point.inode().fs()));
            let target_path = Path::new(receiver, mount_point);

            copy.attach_to_path(target_path, topology);
        }
        self.attach_to_path(target_path, topology);
        pending_changes.commit(topology);

        Ok(())
    }

    /// Gets a child mount node from the mountpoint if any.
    pub(super) fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        self.children.read().get(&mountpoint.key()).cloned()
    }

    /// Gets the root `Dentry` of this mount node.
    pub(in crate::fs) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    /// Gets the mountpoint `Dentry` of this mount node if any.
    pub(in crate::fs) fn mountpoint(&self) -> Option<Arc<Dentry>> {
        self.mountpoint.read().clone()
    }

    /// Sets the mountpoint.
    fn set_mountpoint(&self, dentry: Arc<Dentry>) {
        let mut mountpoint = self.mountpoint.write();
        if let Some(mountpoint) = mountpoint.as_deref() {
            mountpoint.dec_mount_count();
        }

        dentry.inc_mount_count();
        *mountpoint = Some(dentry);
    }

    /// Clears the mountpoint.
    fn clear_mountpoint(&self) {
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
        data: Option<&str>,
        ctx: &Context,
        _topology: &mut MountTopology,
    ) -> Result<()> {
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
    pub(in crate::fs) fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    /// Returns whether `self` is `ancestor` or a descendant of it in the mount tree.
    pub(super) fn is_equal_or_descendant_of(
        &self,
        ancestor: &Arc<Self>,
        _topology: &MountTopology,
    ) -> bool {
        let mut current = self.this();
        loop {
            if Arc::ptr_eq(&current, ancestor) {
                return true;
            }

            let Some(parent) = current.parent().and_then(|parent| parent.upgrade()) else {
                return false;
            };
            current = parent;
        }
    }

    /// Gets the associated mount namespace.
    pub(super) fn mnt_ns(&self) -> &Weak<MountNamespace> {
        &self.mnt_ns
    }

    /// Gets the associated FS.
    pub fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    /// Gets the associated mount flags.
    pub fn flags(&self) -> PerMountFlags {
        self.flags.load(Ordering::Relaxed)
    }

    /// Sets the parent mount node.
    ///
    /// In some cases we may need to reset the parent of
    /// the created Mount, such as move mount.
    fn set_parent(&self, mount: Option<&Arc<Mount>>) {
        let mut parent = self.parent.write();
        *parent = mount.map(Arc::downgrade);
    }

    /// Finds the corresponding `Mount` in the given mount namespace.
    pub(super) fn find_corresponding_mount(
        &self,
        mnt_ns: &Arc<MountNamespace>,
        _topology: &MountTopology,
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
                .cloned()?;
            target_mount = child_mount;
        }

        Some(target_mount)
    }

    pub(super) fn this(&self) -> Arc<Self> {
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
        self.clear_mountpoint();
        // `upgrade` returns `None` for pseudo mounts (no namespace) and for
        // mounts whose namespace is itself being dropped (the `mounts` map
        // will be freed in a moment).
        if let Some(ns) = self.mnt_ns.upgrade() {
            ns.deregister_mount(self.id.unique_id());
        }
        // The recyclable ID is returned to the pool by `MountId`'s `Drop`.
    }
}

/// A policy for cloning a mount tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MountTreeCloneMode {
    /// A bind-mount clone that omits unbindable subtrees.
    Bind,
    /// A namespace clone that omits mounts rooted at mount-namespace files.
    Namespace,
    /// A propagation clone whose mounts are peers of the source mounts.
    SharedPropagation,
    /// A propagation clone whose mounts are slaves of the source mounts.
    SlavePropagation,
}

/// A recyclable mount ID exposed by legacy interfaces such as `/proc/*/mountinfo`.
pub(in crate::fs) type RecyclableMountId = u32;

/// A mount ID that remains unique until reboot.
pub(in crate::fs) type UniqueMountId = u64;

/// 32-bit recyclable mount IDs.
///
/// IDs start at 1; 0 is never issued and denotes an invalid mount.
/// Exhaustion returns `ENOMEM`.
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/namespace.c#L282>
static MOUNT_ID_ALLOCATOR: SpinLock<SparseIdAlloc> =
    SpinLock::new(SparseIdAlloc::new(1, i32::MAX as u32));

/// The first 64-bit unique mount ID.
///
/// The minimum keeps unique IDs above the recyclable ID range, ensuring
/// the two ID spaces never overlap numerically.
///
/// This value is chosen to align with Linux.
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/namespace.c#L73>
pub const MNT_UNIQUE_ID_MIN: u64 = (1 << 31) + 1;

/// Monotonically increasing 64-bit unique mount IDs.
static NEXT_FREE_UNIQUE_ID: AtomicU64 = AtomicU64::new(MNT_UNIQUE_ID_MIN);

pub(super) fn init() {}

/// A mount's pair of identifiers.
///
/// Allocates from [`MOUNT_ID_ALLOCATOR`] and [`NEXT_FREE_UNIQUE_ID`] on
/// construction; on drop, releases the recyclable ID back to the pool.
/// The unique ID is monotonic and is not freed.
pub(super) struct MountId {
    recyclable_id: RecyclableMountId,
    unique_id: UniqueMountId,
}

impl MountId {
    /// Allocates a new pair of mount identifiers.
    ///
    /// Returns `None` if the recyclable ID range is exhausted.
    pub(super) fn alloc() -> Option<Self> {
        let recyclable_id = MOUNT_ID_ALLOCATOR.lock().alloc()?;
        let unique_id = NEXT_FREE_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
        Some(Self {
            recyclable_id,
            unique_id,
        })
    }

    pub(super) fn recyclable_id(&self) -> RecyclableMountId {
        self.recyclable_id
    }

    pub(super) fn unique_id(&self) -> UniqueMountId {
        self.unique_id
    }
}

impl Drop for MountId {
    fn drop(&mut self) {
        MOUNT_ID_ALLOCATOR.lock().free(self.recyclable_id);
    }
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
        /// Do not follow symlinks.
        const NOSYMFOLLOW    = 1 << 8;
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
