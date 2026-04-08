// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use core::{
    fmt::Display,
    str::FromStr,
    sync::atomic::{AtomicU8, Ordering},
};

use aster_systree::{Error, Result, SysAttrSetBuilder, SysBranchNode, SysObj};
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Rcu,
};

use crate::fs::cgroupfs::{
    CgroupMembership, CgroupNode,
    controller::{cpuset::CpuSetController, memory::MemoryController, pids::PidsController},
    systree_node::CgroupSysNode,
};

mod cpuset;
mod memory;
mod pids;

/// A trait to abstract all individual cgroup sub-controllers.
trait SubControl {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize>;

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize>;
}

/// Defines the static properties and behaviors of a specific cgroup sub-controller.
trait SubControlStatic: SubControl + Sized + 'static {
    /// Creates a new instance of the sub-controller.
    fn new(is_root: bool) -> Self;

    /// Returns the `SubCtrlType` enum variant corresponding to this sub-controller.
    fn type_() -> SubCtrlType;

    /// Reads and clones the `Arc` of this sub-controller in the given `Controller`.
    fn read_from(controller: &Controller) -> Arc<SubController<Self>>;
}

/// The type of a sub-controller in the cgroup subsystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SubCtrlType {
    Memory,
    CpuSet,
    Pids,
}

impl SubCtrlType {
    const ALL: [Self; 3] = [Self::Memory, Self::CpuSet, Self::Pids];
}

impl FromStr for SubCtrlType {
    type Err = aster_systree::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "memory" => Ok(SubCtrlType::Memory),
            "cpuset" => Ok(SubCtrlType::CpuSet),
            "pids" => Ok(SubCtrlType::Pids),
            _ => Err(Error::NotFound),
        }
    }
}

bitflags! {
    /// A set of sub-controller types, represented as bitflags.
    pub(super) struct SubCtrlSet: u8 {
        const MEMORY = 1 << 0;
        const CPUSET = 1 << 1;
        const PIDS = 1 << 2;
    }
}

impl SubCtrlSet {
    /// Checks whether a sub-control is active in the current set.
    pub(super) fn contains_type(&self, ctrl_type: SubCtrlType) -> bool {
        self.contains(ctrl_type.into())
    }

    /// Adds a sub-control type to the current set.
    pub(super) fn add_type(&mut self, ctrl_type: SubCtrlType) {
        *self |= ctrl_type.into()
    }

    /// Removes a sub-control type from the current set.
    pub(super) fn remove_type(&mut self, ctrl_type: SubCtrlType) {
        *self -= ctrl_type.into()
    }

    /// Returns an iterator over the sub-controller types in the current set.
    pub(super) fn iter_types(&self) -> impl Iterator<Item = SubCtrlType> + '_ {
        SubCtrlType::ALL
            .into_iter()
            .filter(|&ctrl_type| self.contains_type(ctrl_type))
    }
}

impl Display for SubCtrlSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.contains(Self::MEMORY) {
            write!(f, "memory ")?;
        }
        if self.contains(Self::CPUSET) {
            write!(f, "cpuset ")?;
        }
        if self.contains(Self::PIDS) {
            write!(f, "pids")?;
        }

        Ok(())
    }
}

impl From<SubCtrlType> for SubCtrlSet {
    fn from(ctrl_type: SubCtrlType) -> Self {
        match ctrl_type {
            SubCtrlType::Memory => Self::MEMORY,
            SubCtrlType::CpuSet => Self::CPUSET,
            SubCtrlType::Pids => Self::PIDS,
        }
    }
}

impl From<u8> for SubCtrlSet {
    fn from(value: u8) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<SubCtrlSet> for u8 {
    fn from(value: SubCtrlSet) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(SubCtrlSet, {
    /// An atomic version of `SubCtrlSet`.
    #[derive(Debug)]
    struct AtomicSubCtrlSet(AtomicU8);
});

/// The sub-controller for a specific cgroup controller type.
///
/// If the sub-controller is inactive, the `inner` field will be `None`.
struct SubController<T: SubControlStatic> {
    inner: Option<T>,
    /// The parent sub-controller in the hierarchy.
    ///
    /// This field is used to traverse the controller hierarchy.
    parent: Option<Arc<SubController<T>>>,
}

impl<T: SubControlStatic> SubController<T> {
    fn new(parent_controller: Option<&Controller>) -> Self {
        let is_active = if let Some(parent) = parent_controller {
            parent.active_set().contains_type(T::type_())
        } else {
            true
        };

        let inner = if is_active {
            Some(T::new(parent_controller.is_none()))
        } else {
            None
        };

        let parent = parent_controller.map(T::read_from);

        Self { inner, parent }
    }
}

trait TryGetSubControl {
    fn try_get(&self) -> Option<&dyn SubControl>;
}

impl<T: SubControlStatic> TryGetSubControl for SubController<T> {
    fn try_get(&self) -> Option<&dyn SubControl> {
        self.inner.as_ref().map(|sub_ctrl| sub_ctrl as _)
    }
}

/// The controller for a single cgroup.
///
/// This struct can manage the activation state of each sub-control, and dispatches read/write
/// operations to the appropriate sub-controllers.
///
/// The following is an explanation of the activation for sub-controls and sub-controllers. When a
/// cgroup activates a specific sub-control (e.g., memory, io), it means this control capability is
/// being delegated to its children. Consequently, the corresponding sub-controller within the
/// child nodes will be activated.
///
/// The root node serves as the origin for all these control capabilities, so the sub-controllers
/// it possesses are always active. For any other node, only if its parent node first enables a
/// sub-control, its corresponding sub-controller will be activated.
pub struct Controller {
    /// A set of types of active sub-controllers.
    ///
    /// Updates to this set are serialized by `CgroupMembership::write_lock()`.
    active_set: AtomicSubCtrlSet,

    memory: Rcu<Arc<SubController<MemoryController>>>,
    cpuset: Rcu<Arc<SubController<CpuSetController>>>,
    pids: Rcu<Arc<SubController<PidsController>>>,
}

impl Controller {
    /// Creates a new controller manager for a cgroup.
    pub(super) fn new(parent_controller: Option<&Controller>) -> Self {
        let memory_controller = Arc::new(SubController::new(parent_controller));
        let cpuset_controller = Arc::new(SubController::new(parent_controller));
        let pids_controller = Arc::new(SubController::new(parent_controller));

        Self {
            active_set: AtomicSubCtrlSet::new(SubCtrlSet::empty()),
            memory: Rcu::new(memory_controller),
            cpuset: Rcu::new(cpuset_controller),
            pids: Rcu::new(pids_controller),
        }
    }

    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        MemoryController::init_attr_set(builder, is_root);
        CpuSetController::init_attr_set(builder, is_root);
        PidsController::init_attr_set(builder, is_root);
    }

    fn read_sub(&self, ctrl_type: SubCtrlType) -> Arc<dyn TryGetSubControl> {
        match ctrl_type {
            SubCtrlType::Memory => MemoryController::read_from(self),
            SubCtrlType::CpuSet => CpuSetController::read_from(self),
            SubCtrlType::Pids => PidsController::read_from(self),
        }
    }

    /// Returns whether the attribute with the given name is absent in this controller.
    pub(super) fn is_attr_absent(&self, name: &str) -> bool {
        let Some((subsys, _)) = name.split_once('.') else {
            return false;
        };
        let Ok(ctrl_type) = SubCtrlType::from_str(subsys) else {
            return false;
        };

        let sub_controller = self.read_sub(ctrl_type);
        if sub_controller.try_get().is_none() {
            // If the sub-controller is not active, all its attributes are considered absent.
            true
        } else {
            false
        }
    }

    pub(super) fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };
        let ctrl_type = SubCtrlType::from_str(subsys)?;

        let sub_controller = self.read_sub(ctrl_type);
        let Some(controller) = sub_controller.try_get() else {
            return Err(Error::IsDead);
        };

        controller.read_attr_at(name, offset, writer)
    }

    pub(super) fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };
        let ctrl_type = SubCtrlType::from_str(subsys)?;

        let sub_controller = self.read_sub(ctrl_type);
        let Some(controller) = sub_controller.try_get() else {
            return Err(Error::IsDead);
        };

        controller.write_attr(name, reader)
    }

    /// Activates a sub-control of the specified type.
    pub(super) fn activate(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        parent_controller: Option<&Controller>,
        cgroup_membership: &mut CgroupMembership,
    ) -> Result<()> {
        let mut active_set = self.active_set();
        if active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        // A cgroup can activate the sub-control only if this
        // sub-control has been activated in its parent cgroup.
        if parent_controller
            .is_some_and(|controller| !controller.active_set().contains_type(ctrl_type))
        {
            return Err(Error::NotFound);
        }

        active_set.add_type(ctrl_type);
        self.active_set.store(active_set, Ordering::Relaxed);
        self.update_sub_controllers_for_descents(ctrl_type, current_node, cgroup_membership);

        Ok(())
    }

    /// Deactivates a sub-control of the specified type.
    pub(super) fn deactivate(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        cgroup_membership: &mut CgroupMembership,
    ) -> Result<()> {
        let mut active_set = self.active_set();
        if !active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        // If any child node has activated this sub-control,
        // the deactivation operation will be rejected.
        for child in current_node.children() {
            let cgroup_child = Arc::downcast::<CgroupNode>(child).unwrap();
            // This is race-free because if a child wants to activate a sub-controller, it must
            // acquire the write lock of `CgroupMembership` which has already been held here.
            if cgroup_child
                .controller()
                .active_set()
                .contains_type(ctrl_type)
            {
                return Err(Error::InvalidOperation);
            }
        }

        active_set.remove_type(ctrl_type);
        self.active_set.store(active_set, Ordering::Relaxed);
        self.update_sub_controllers_for_descents(ctrl_type, current_node, cgroup_membership);

        Ok(())
    }

    fn update_sub_controllers_for_descents(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        cgroup_membership: &mut CgroupMembership,
    ) {
        fn update_sub_controller_for_one_child(
            child: &Arc<dyn SysObj>,
            ctrl_type: SubCtrlType,
            parent_controller: &Controller,
            cgroup_membership: &mut CgroupMembership,
        ) {
            let child_node = child.as_any().downcast_ref::<CgroupNode>().unwrap();
            match ctrl_type {
                SubCtrlType::Memory => {
                    let new_controller = Arc::new(SubController::new(Some(parent_controller)));
                    child_node.controller().memory.update(new_controller);
                }
                SubCtrlType::CpuSet => {
                    let new_controller = Arc::new(SubController::new(Some(parent_controller)));
                    child_node.controller().cpuset.update(new_controller);
                }
                SubCtrlType::Pids => {
                    let mut new_controller: SubController<PidsController> =
                        SubController::new(Some(parent_controller));
                    if let Some(inner) = new_controller.inner.as_mut() {
                        // When the pids sub-controller is being activated, initialize
                        // `pids.current` with the number of processes already present
                        // in this cgroup's subtree. The parent's counter is already
                        // correct because charges propagated through inactive levels.
                        let count = cgroup_membership.count_subtree_processes(child_node);
                        if count > 0 {
                            inner.init_count(count);
                        }
                    }
                    child_node
                        .controller()
                        .pids
                        .update(Arc::new(new_controller));
                }
            }
        }

        let mut descents = VecDeque::new();

        // Subtree-control writes hold `CgroupMembership::write_lock()`, so operations
        // protected by `CgroupMembership` like controller activation and deactivation
        // and child node creation are serialized while the subtree updates are applied.
        //
        // Concurrent child removal is still fine: If a child is removed, we may update
        // a sub-controller that's about to be destroyed, which is harmless.

        // Update the direct children first.
        current_node.visit_children_with(0, &mut |child_node| {
            descents.push_back(child_node.clone());
            update_sub_controller_for_one_child(child_node, ctrl_type, self, cgroup_membership);

            Some(())
        });

        // Then update all the other descendent nodes.
        while let Some(node) = descents.pop_front() {
            let current_node = Arc::downcast::<CgroupNode>(node).unwrap();
            let current_controller = current_node.controller();
            current_node.visit_children_with(0, &mut |child_node| {
                descents.push_back(child_node.clone());
                update_sub_controller_for_one_child(
                    child_node,
                    ctrl_type,
                    current_controller,
                    cgroup_membership,
                );

                Some(())
            });
        }
    }

    pub(super) fn active_set(&self) -> SubCtrlSet {
        self.active_set.load(Ordering::Relaxed)
    }
}

// For pids sub-controller
impl Controller {
    /// Charges a process in the pids sub-controller hierarchy.
    ///
    /// This operation is used for explicit migration and will not enforce
    /// the `pids.max` limit.
    ///
    /// Reference: <https://docs.kernel.org/admin-guide/cgroup-v2.html#pid>
    pub(super) fn charge_pids(&self) {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.charge_hierarchy();
    }

    /// Pre-charges a process in the pids sub-controller hierarchy,
    /// enforcing `pids.max` at each level.
    ///
    /// This is used at fork time. Returns a guard that will roll back the
    /// charge unless it is explicitly applied.
    ///
    /// Note that concurrent operations of moving processes to cgroups and
    /// setting `pids.max` may cause the actual charge to exceed the peak at
    /// charge time.
    pub fn pre_charge_pids<'a>(
        &'a self,
        _cgroup_membership: &'a CgroupMembership,
    ) -> core::result::Result<PidsPreCharge<'a>, TryChargeError> {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.try_charge_hierarchy()?;

        // This will not outlive `_cgroup_membership`, so we will roll back
        // the charge for the right pids sub-controller.
        Ok(PidsPreCharge { controller: self })
    }

    /// Uncharges a process in the pids sub-controller hierarchy.
    pub(super) fn uncharge_pids(&self) {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.uncharge_hierarchy();
    }
}

/// Represents a pre-charged pids slot that rolls back on drop.
pub struct PidsPreCharge<'a> {
    controller: &'a Controller,
}

impl PidsPreCharge<'_> {
    /// Applies the pre-charge and prevents rollback on drop.
    pub(super) fn apply(self) {
        core::mem::forget(self);
    }
}

impl Drop for PidsPreCharge<'_> {
    fn drop(&mut self) {
        self.controller.uncharge_pids();
    }
}

/// An error type indicating that a problem occurred during the charge operation.
#[derive(Clone, Copy, Debug)]
pub struct TryChargeError;
