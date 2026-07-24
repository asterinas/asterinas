// SPDX-License-Identifier: MPL-2.0

//! Mount-event propagation state and topology.
//!
//! [`MountPropagation`] describes how one mount participates in propagation.
//! Fallible operations prepare new relationships in [`PendingPropagationChanges`]
//! and publish them only after the enclosing tree operation succeeds.
//! [`MountTopology`] serializes mount-tree changes and indexes the peer and
//! master relationships used to route propagation events.

use hashbrown::HashSet;
use ostd::sync::{RwMutexReadGuard, RwMutexWriteGuard};
use sparse_id_alloc::SparseIdAlloc;

use super::{UniqueMountId, dentry::Dentry, mount::Mount};
use crate::prelude::*;

/// The internal mount event propagation state of one mount.
///
/// A mount event is the attachment or detachment of a child mount at a dentry below a
/// parent mount. The parent mount's propagation state determines whether the event is
/// replicated to other mounts; the propagation state of the child does not.
///
/// Mount propagation connects mount objects,
/// possibly in different mount namespaces,
/// so that a mount event below one object can cause corresponding operations below others.
/// Only mount-tree changes are propagated;
/// file operations and filesystem contents are not.
///
/// # State model
///
/// A propagation state combines three fields with distinct roles:
///
/// - **Own propagation group.**
///   `peer_group_id = Some(G)` places the mount in peer group G.
///   An event originating at or delivered to a member of G is replicated to the other peers
///   and then to mounts directly slaved to G.
///   Thus G is the domain through which this mount both receives and forwards events.
/// - **Upstream propagation source.**
///   `master_id = Some(M)` adds a directed edge from peer group M to this mount.
///   Events from M enter the mount and,
///   if the mount also owns a peer group G,
///   continue through G.
///   Events never travel from the mount back to M along this edge.
/// - **Bindability.**
///   `bindability` controls whether this mount may be bind-cloned.
///   It does not add a propagation edge.
///
/// G is the mount's own peer-group ID,
/// while M is a distinct upstream peer-group ID.
/// Both refer to groups indexed by [`MountTopology`].
///
/// ## Encoded states
///
/// | Peer group | Master | Bindability | State and behavior |
/// |---|---|---|---|
/// | `None` | `None` | Bindable | Private; neither sends nor receives events. |
/// | `None` | `Some(M)` | Bindable | Slave; receives events from M. |
/// | `Some(G)` | `None` | Bindable | Shared; exchanges events within G. |
/// | `Some(G)` | `Some(M)` | Bindable | Shared-and-slave; exchanges events within G and receives from M. |
/// | `None` | `None` | Unbindable | Private and cannot be bind-cloned. |
///
/// An unbindable state with a peer group or master is not constructed.
///
/// # Copying mount state
///
/// Propagation state belongs to a mount object,
/// rather than to its filesystem or root dentry.
/// Creating another mount object derives its initial state from the operation:
///
/// | Operation | Initial state of each created mount |
/// |---|---|
/// | New filesystem mount | Private. |
/// | Bind clone | The source state. Shared clones remain in G, and slave clones retain M. An unbindable root is rejected; recursive cloning prunes unbindable subtrees. |
/// | Mount-namespace clone | The source state, including G, M, and unbindability. |
///
/// # Applying an event
///
/// | Event | Receiver | Effect |
/// |---|---|---|
/// | Attach | Peer | Clone the attached mount tree; corresponding mounts become peers. |
/// | Attach | Slave | Clone the attached mount tree; each copy is slaved to its source's peer group. |
/// | Unmount | Any receiver | Detach the corresponding child if it exists. |
///
/// An attach copy needs a source peer group,
/// so each corresponding source mount is made shared if necessary.
/// When a slave receiver is also shared,
/// its copy forms the source of a new shared layer,
/// and the event continues to that layer's downstream slaves.
///
/// Attaching the created tree may refine its initial state:
///
/// - Attaching below a non-shared parent preserves the tree's state.
/// - Attaching below a shared parent makes every mount in the attached tree shared,
///   while preserving any existing master relationship.
///
/// # State transitions
///
/// The transition table uses this notation:
///
/// - `P`: private;
/// - `U`: unbindable;
/// - `S(M)`: slave of M;
/// - `SH(G)`: shared in G; and
/// - `SH(G, M)`: shared in G and slave of M.
///
/// | Before | Make shared | Make slave | Make private | Make unbindable |
/// |---|---|---|---|---|
/// | `P` | `SH(new)` | `P` | `P` | `U` |
/// | `U` | `SH(new)` | `U` | `P` | `U` |
/// | `S(M)` | `SH(new, M)` | `S(M)` | `P` | `U` |
/// | `SH(G)` | Unchanged | `S(G)` or `P` (1) | `P` | `U` |
/// | `SH(G, M)` | Unchanged | `S(G)` or `S(M)` (2) | `P` | `U` |
///
/// 1. If other peers remain in G,
///    the mount becomes their slave, `S(G)`.
///    If it was G's last peer,
///    G disappears and the mount becomes private.
/// 2. If other peers remain in G,
///    the mount becomes `S(G)`.
///    If it was G's last peer,
///    G disappears and the mount falls back to its former upstream master as `S(M)`.
///
/// Making a mount private always restores bindability.
/// Making it unbindable also makes it private,
/// but prevents it from being the root of a bind clone.
/// Making a private or unbindable mount a slave has no effect
/// because there is no peer group that can become its master.
///
/// A recursive policy change applies this table independently
/// to every mount in the selected subtree.
/// It does not place the whole subtree into one peer group.
///
/// # Removing a peer group
///
/// A group is removed when its last peer leaves because of a policy change
/// or because the mount itself is removed.
/// An empty group cannot remain a master,
/// so each of its direct slaves is reparented as follows:
///
/// | Direct slave before removal | Group has upstream M | No upstream master |
/// |---|---|---|
/// | `S(G)` | `S(M)` | `P` |
/// | `SH(H, G)` | `SH(H, M)` | `SH(H)` |
///
/// Reparenting preserves the existing downstream propagation direction
/// without leaving references to an empty peer group.
///
/// These rules implement Linux shared-subtree semantics.
/// See the Linux kernel's [Shared Subtrees] documentation for the
/// corresponding user-space model.
///
/// [Shared Subtrees]: https://docs.kernel.org/filesystems/sharedsubtree.html
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MountPropagation {
    peer_group_id: Option<PeerGroupId>,
    master_id: Option<PeerGroupId>,
    bindability: MountBindability,
}

/// A recyclable identifier for a live mount peer group.
pub(super) type PeerGroupId = u32;

impl Default for MountPropagation {
    fn default() -> Self {
        Self::private()
    }
}

impl MountPropagation {
    pub(super) fn private() -> Self {
        Self {
            peer_group_id: None,
            master_id: None,
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn slave(master_id: PeerGroupId) -> Self {
        Self {
            peer_group_id: None,
            master_id: Some(master_id),
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn shared(peer_group_id: PeerGroupId, master_id: Option<PeerGroupId>) -> Self {
        debug_assert_ne!(master_id, Some(peer_group_id));
        Self {
            peer_group_id: Some(peer_group_id),
            master_id,
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn unbindable() -> Self {
        Self {
            peer_group_id: None,
            master_id: None,
            bindability: MountBindability::Unbindable,
        }
    }

    pub(super) fn peer_group_id(&self) -> Option<PeerGroupId> {
        self.peer_group_id
    }

    pub(super) fn master_id(&self) -> Option<PeerGroupId> {
        self.master_id
    }

    pub(super) fn is_shared(&self) -> bool {
        self.peer_group_id.is_some()
    }

    pub(super) fn is_unbindable(&self) -> bool {
        self.bindability == MountBindability::Unbindable
    }

    pub(super) fn set_master_id(&mut self, master_id: Option<PeerGroupId>) {
        self.master_id = master_id;
    }

    /// Converts this propagation state to shared.
    ///
    /// Private and unbindable mounts get a new peer group. Plain slaves get a
    /// new peer group while keeping their master, becoming shared-and-slave.
    /// An already shared state is unchanged.
    pub(super) fn make_shared(self, peer_group_id: PeerGroupId) -> Self {
        if self.is_shared() {
            return self;
        }

        Self::shared(peer_group_id, self.master_id())
    }

    /// Converts this propagation state from shared to slave.
    ///
    /// A non-shared state is unchanged.
    /// A shared mount initially names its old peer group as its master.
    /// If removing the mount empties that group,
    /// [`MountTopology::remove_peer`] replaces this temporary relationship with
    /// the removed group's upstream master.
    pub(super) fn make_slave(self) -> Self {
        let Some(peer_group_id) = self.peer_group_id() else {
            return self;
        };

        Self::slave(peer_group_id)
    }
}

/// Whether a mount can be bind-mounted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MountBindability {
    #[default]
    Bindable,
    Unbindable,
}

/// A mount propagation policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MountPropType {
    /// A mount that neither sends nor receives propagation events.
    #[default]
    Private,
    /// A mount that receives events from a master peer group without sending
    /// local events back to that group.
    ///
    /// Making a slave shared forms the internal shared-and-slave state.
    Slave,
    /// A mount that sends events to its peers and downstream slaves.
    ///
    /// Events received from another peer or an upstream master are forwarded
    /// through the same relationships.
    Shared,
    /// A private mount that cannot be used as the root of a bind clone.
    Unbindable,
}

/// The synchronized index of mount peer groups and propagation relationships.
///
/// Parent-child links and the propagation indexes must be accessed under the
/// same global lock so event propagation observes one consistent topology.
pub(in crate::fs) struct MountTopology {
    peer_groups: BTreeMap<PeerGroupId, PeerGroup>,
}

static MOUNT_TOPOLOGY: RwMutex<MountTopology> = RwMutex::new(MountTopology {
    peer_groups: BTreeMap::new(),
});

impl MountTopology {
    /// Acquires the write side of the mount topology lock.
    pub(super) fn write_lock() -> RwMutexWriteGuard<'static, Self> {
        MOUNT_TOPOLOGY.write()
    }

    /// Acquires the read side of the mount topology lock.
    pub(super) fn read_lock() -> RwMutexReadGuard<'static, Self> {
        MOUNT_TOPOLOGY.read()
    }

    /// Collects every propagation receiver that can address `mountpoint`.
    ///
    /// A receiver participates only if `mountpoint` is at or below its root dentry;
    /// a bind mount rooted elsewhere cannot address the event location.
    pub(super) fn propagation_receivers(
        &self,
        source_parent: &Arc<Mount>,
        mountpoint: &Arc<Dentry>,
    ) -> Vec<Arc<Mount>> {
        let mut visited = HashSet::new();
        visited.insert(source_parent.unique_id());
        let mut try_visit_receiver = |receiver: &Arc<Mount>| {
            visited.insert(receiver.unique_id())
                && mountpoint.is_equal_or_descendant_of(receiver.root_dentry())
        };

        let mut receivers = self
            .direct_propagation_receivers(source_parent)
            .filter(&mut try_visit_receiver)
            .collect::<Vec<_>>();

        let mut work_idx = 0;
        while work_idx < receivers.len() {
            let direct_receivers = self.direct_propagation_receivers(&receivers[work_idx]);
            receivers.extend(direct_receivers.filter(&mut try_visit_receiver));
            work_idx += 1;
        }

        receivers
    }

    /// Iterates over the live peers in `peer_group_id`.
    pub(super) fn group_peers_of(
        &self,
        peer_group_id: PeerGroupId,
    ) -> impl Iterator<Item = Arc<Mount>> + '_ {
        self.peer_groups
            .get(&peer_group_id)
            .into_iter()
            .flat_map(|group| group.peers.values())
            .filter_map(Weak::upgrade)
    }

    /// Iterates over the live mounts directly slaved to `peer_group_id`.
    pub(super) fn slave_mounts_of(
        &self,
        peer_group_id: PeerGroupId,
    ) -> impl Iterator<Item = Arc<Mount>> + '_ {
        self.peer_groups
            .get(&peer_group_id)
            .into_iter()
            .flat_map(|group| group.slaves.values())
            .filter_map(Weak::upgrade)
    }

    /// Removes a peer and repairs downstream relationships if its group becomes empty.
    fn remove_peer(
        &mut self,
        peer_id: UniqueMountId,
        propagation: &MountPropagation,
    ) -> Option<PeerGroupId> {
        let peer_group_id = propagation
            .peer_group_id()
            .expect("peer propagation must have a peer group");
        let upstream_master_id = propagation.master_id();
        let peer_group = self.peer_groups.get_mut(&peer_group_id)?;
        let removed_peer = peer_group.peers.remove(&peer_id);
        debug_assert!(removed_peer.is_some());

        if !peer_group.peers.is_empty() {
            return None;
        }

        // An empty group cannot remain a master. Reparent its direct slaves
        // before dropping the group and releasing its ID.
        let mut peer_group = self
            .peer_groups
            .remove(&peer_group_id)
            .expect("peer group must still exist");
        debug_assert!(peer_group.peers.is_empty());

        for (mount_id, slave) in core::mem::take(&mut peer_group.slaves) {
            let Some(slave_mount) = slave.upgrade() else {
                continue;
            };

            slave_mount.set_propagation_master(upstream_master_id);
            if let Some(upstream_master_id) = upstream_master_id {
                let upstream_group = self
                    .peer_groups
                    .get_mut(&upstream_master_id)
                    .expect("upstream master peer group must exist");
                let old_slave = upstream_group.slaves.insert(mount_id, slave);
                debug_assert!(old_slave.is_none());
            }
        }

        upstream_master_id
    }

    fn insert_peer_group(&mut self, peer_group: PeerGroup) {
        let peer_group_id = peer_group.id;
        let old_group = self.peer_groups.insert(peer_group_id, peer_group);
        debug_assert!(old_group.is_none());
    }

    /// Registers the peer and master relationships of a newly published mount.
    fn register_mount_propagation(&mut self, mount: &Arc<Mount>) {
        let propagation = mount.propagation();

        if let Some(peer_group_id) = propagation.peer_group_id() {
            let peer_group = self
                .peer_groups
                .get_mut(&peer_group_id)
                .expect("peer group must exist");
            let old_peer = peer_group
                .peers
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_peer.is_none());
        }

        if let Some(master_id) = propagation.master_id() {
            let master_group = self
                .peer_groups
                .get_mut(&master_id)
                .expect("master peer group must exist");
            let old_slave = master_group
                .slaves
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_slave.is_none());
        }
    }

    /// Replaces a mount's propagation relationships in the topology.
    ///
    /// If removing the mount's last peer destroys its old group, the returned state
    /// reparents the mount to that group's upstream master.
    pub(super) fn replace_mount_propagation(
        &mut self,
        mount: &Arc<Mount>,
        old: MountPropagation,
        mut new: MountPropagation,
    ) -> MountPropagation {
        let mut old_master_removed = false;
        if old.master_id() != new.master_id()
            && let Some(old_master_id) = old.master_id()
        {
            let old_master_group = self
                .peer_groups
                .get_mut(&old_master_id)
                .expect("master peer group must exist");
            let old_slave = old_master_group.slaves.remove(&mount.unique_id());
            debug_assert!(old_slave.is_some());
            old_master_removed = true;
        }

        if old.peer_group_id() != new.peer_group_id()
            && let Some(old_peer_group_id) = old.peer_group_id()
        {
            let old_group_existed = self.peer_groups.contains_key(&old_peer_group_id);
            let reparented_master_id = self.remove_peer(mount.unique_id(), &old);
            if old_group_existed
                && !self.peer_groups.contains_key(&old_peer_group_id)
                && new.master_id() == Some(old_peer_group_id)
            {
                new.set_master_id(reparented_master_id);
            }
        }

        if let Some(new_master_id) = new.master_id()
            && (old_master_removed || old.master_id() != Some(new_master_id))
        {
            let new_master_group = self
                .peer_groups
                .get_mut(&new_master_id)
                .expect("master peer group must exist");
            let old_slave = new_master_group
                .slaves
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_slave.is_none());
        }

        if let Some(new_peer_group_id) = new.peer_group_id()
            && old.peer_group_id() != Some(new_peer_group_id)
        {
            let new_peer_group = self
                .peer_groups
                .get_mut(&new_peer_group_id)
                .expect("peer group must exist");
            let old_peer = new_peer_group
                .peers
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_peer.is_none());
        }

        new
    }

    /// Removes a mount's propagation relationships from the topology.
    pub(super) fn unregister_mount_propagation(&mut self, mount: &Mount, old: MountPropagation) {
        if let Some(old_master_id) = old.master_id() {
            let old_master_group = self
                .peer_groups
                .get_mut(&old_master_id)
                .expect("master peer group must exist");
            let old_slave = old_master_group.slaves.remove(&mount.unique_id());
            debug_assert!(old_slave.is_some());
        }

        if old.peer_group_id().is_some() {
            self.remove_peer(mount.unique_id(), &old);
        }
    }

    /// Iterates over the other peers and direct slaves of `source_parent`.
    fn direct_propagation_receivers<'a>(
        &'a self,
        source_parent: &Mount,
    ) -> impl Iterator<Item = Arc<Mount>> + 'a {
        let source_parent_id = source_parent.unique_id();
        let peer_group_id = source_parent.propagation().peer_group_id();
        let peers = peer_group_id
            .into_iter()
            .flat_map(|peer_group_id| self.group_peers_of(peer_group_id))
            .filter(move |peer| peer.unique_id() != source_parent_id);
        let slaves = peer_group_id
            .into_iter()
            .flat_map(|peer_group_id| self.slave_mounts_of(peer_group_id));

        peers.chain(slaves)
    }
}

/// A batch of mount-propagation changes awaiting publication.
///
/// Staged changes are not visible until the batch is committed. Dropping the batch discards
/// all of them, allowing fallible mount operations to publish their propagation changes on an
/// all-or-nothing basis.
#[derive(Default)]
pub(super) struct PendingPropagationChanges {
    created_peer_groups: BTreeMap<UniqueMountId, PeerGroup>,
    propagation_ops: Vec<MountPropagationOp>,
}

impl PendingPropagationChanges {
    /// Returns the shared state used to create a peer clone of `source_mount`.
    pub(super) fn make_shared_for_clone(
        &mut self,
        source_mount: &Mount,
    ) -> Result<MountPropagation> {
        let current_propagation = source_mount.propagation();
        if current_propagation.is_shared() {
            return Ok(current_propagation);
        }

        if let Some(peer_group) = self.created_peer_groups.get(&source_mount.unique_id()) {
            return Ok(current_propagation.make_shared(peer_group.id));
        }

        let source_mount = source_mount.this();
        let peer_group = PeerGroup::new()?;
        let new_propagation = current_propagation.make_shared(peer_group.id);
        self.add_peer_group(source_mount.unique_id(), peer_group);
        self.add_set_op(source_mount, new_propagation);
        Ok(new_propagation)
    }

    pub(super) fn add_make_slave_op(&mut self, mount: Arc<Mount>) {
        self.propagation_ops
            .push(MountPropagationOp::MakeSlave { mount });
    }

    pub(super) fn add_set_op(&mut self, mount: Arc<Mount>, new_propagation: MountPropagation) {
        self.propagation_ops.push(MountPropagationOp::Set {
            mount,
            propagation: new_propagation,
        });
    }

    pub(super) fn add_register_op(&mut self, created_mount: Arc<Mount>) {
        self.propagation_ops
            .push(MountPropagationOp::Register(created_mount));
    }

    /// Publishes all pending changes to the mount propagation topology.
    ///
    /// This is an infallible commit step. The caller must have completed every fallible part of
    /// the enclosing operation and must hold the mount-topology write lock.
    pub(super) fn commit(mut self, topology: &mut MountTopology) {
        for (_, peer_group) in core::mem::take(&mut self.created_peer_groups) {
            topology.insert_peer_group(peer_group);
        }

        for operation in self.propagation_ops.drain(..) {
            operation.commit(topology);
        }
    }

    pub(super) fn add_peer_group(
        &mut self,
        mount_id: UniqueMountId,
        created_peer_group: PeerGroup,
    ) {
        let old_peer_group = self
            .created_peer_groups
            .insert(mount_id, created_peer_group);
        debug_assert!(old_peer_group.is_none());
    }
}

/// A propagation-topology operation deferred until commit.
enum MountPropagationOp {
    Set {
        mount: Arc<Mount>,
        propagation: MountPropagation,
    },
    MakeSlave {
        mount: Arc<Mount>,
    },
    Register(Arc<Mount>),
}

impl MountPropagationOp {
    fn commit(self, topology: &mut MountTopology) {
        match self {
            Self::Set { mount, propagation } => {
                mount.publish_propagation(propagation, topology);
            }
            Self::MakeSlave { mount } => {
                let current_propagation = mount.propagation();
                mount.publish_propagation(current_propagation.make_slave(), topology);
            }
            Self::Register(mount) => topology.register_mount_propagation(&mount),
        }
    }
}

/// A live mount peer group protected by [`MountTopology`].
pub(super) struct PeerGroup {
    id: PeerGroupId,
    // Unique mount IDs provide stable keys even when the weak reference cannot be upgraded.
    peers: BTreeMap<UniqueMountId, Weak<Mount>>,
    slaves: BTreeMap<UniqueMountId, Weak<Mount>>,
}

impl PeerGroup {
    pub(super) fn id(&self) -> PeerGroupId {
        self.id
    }

    pub(super) fn new() -> Result<Self> {
        let id = PEER_GROUP_ID_ALLOCATOR
            .lock()
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "peer group ID pool exhausted"))?;

        Ok(Self {
            id,
            peers: BTreeMap::new(),
            slaves: BTreeMap::new(),
        })
    }
}

impl Drop for PeerGroup {
    fn drop(&mut self) {
        PEER_GROUP_ID_ALLOCATOR.lock().free(self.id);
    }
}

/// The first peer-group ID; zero denotes no peer group.
const PEER_GROUP_ID_MIN: PeerGroupId = 1;

/// The last peer-group ID.
const PEER_GROUP_ID_MAX: PeerGroupId = i32::MAX as PeerGroupId;

/// The allocator for recyclable IDs of live mount peer groups.
///
/// Like Linux, IDs start at one because zero denotes the absence of a peer
/// group, and an ID is returned to the allocator when its group is destroyed.
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/namespace.c#L295>
static PEER_GROUP_ID_ALLOCATOR: SpinLock<SparseIdAlloc> =
    SpinLock::new(SparseIdAlloc::new(PEER_GROUP_ID_MIN, PEER_GROUP_ID_MAX));
