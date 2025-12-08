// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use core::{fmt::Display, str::FromStr};

use aster_systree::{Error, Result, SysAttrSetBuilder, SysBranchNode, SysObj};
use bitflags::bitflags;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::{Mutex, MutexGuard, Rcu},
};

use crate::fs::cgroupfs::{
    CgroupNode,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The sub-controller for a specific cgroup controller type.
///
/// If the sub-controller is inactive, the `inner` field will be `None`.
struct SubController<T: SubControlStatic> {
    inner: Option<T>,
    /// The parent sub-controller in the hierarchy.
    ///
    /// This field is used to traverse the controller hierarchy.
    #[expect(dead_code)]
    parent: Option<Arc<SubController<T>>>,
}

impl<T: SubControlStatic> SubController<T> {
    fn new(parent_controller: Option<&LockedController>) -> Arc<Self> {
        let is_active = if let Some(parent) = parent_controller {
            parent.active_set.contains_type(T::type_())
        } else {
            true
        };

        let inner = if is_active {
            Some(T::new(parent_controller.is_none()))
        } else {
            None
        };

        let parent = parent_controller.map(|controller| T::read_from(controller.controller));

        Arc::new(Self { inner, parent })
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
pub(super) struct Controller {
    /// A set of types of active sub-controllers.
    active_set: Mutex<SubCtrlSet>,

    memory: Rcu<Arc<SubController<MemoryController>>>,
    cpuset: Rcu<Arc<SubController<CpuSetController>>>,
    pids: Rcu<Arc<SubController<PidsController>>>,
}

impl Controller {
    /// Creates a new controller manager for a cgroup.
    pub(super) fn new(locked_parent_controller: Option<&LockedController>) -> Self {
        let memory_controller = SubController::new(locked_parent_controller);
        let cpuset_controller = SubController::new(locked_parent_controller);
        let pids_controller = SubController::new(locked_parent_controller);

        Self {
            active_set: Mutex::new(SubCtrlSet::empty()),
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

    pub(super) fn lock(&self) -> LockedController {
        LockedController {
            active_set: self.active_set.lock(),
            controller: self,
        }
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
}

/// A locked controller for a cgroup.
///
/// Holding this lock indicates exclusive access to modify the sub-control state.
pub(super) struct LockedController<'a> {
    active_set: MutexGuard<'a, SubCtrlSet>,
    controller: &'a Controller,
}

impl LockedController<'_> {
    /// Activates a sub-control of the specified type.
    pub(super) fn activate(
        &mut self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        parent_controller: Option<&LockedController>,
    ) -> Result<()> {
        if self.active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        // A cgroup can activate the sub-control only if this
        // sub-control has been activated in its parent cgroup.
        if parent_controller
            .is_some_and(|controller| !controller.active_set.contains_type(ctrl_type))
        {
            return Err(Error::NotFound);
        }

        self.active_set.add_type(ctrl_type);
        self.update_sub_controllers_for_descents(ctrl_type, current_node);

        Ok(())
    }

    /// Deactivates a sub-control of the specified type.
    pub(super) fn deactivate(
        &mut self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
    ) -> Result<()> {
        if !self.active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        // If any child node has activated this sub-control,
        // the deactivation operation will be rejected.
        for child in current_node.children() {
            let cgroup_child = child.as_any().downcast_ref::<CgroupNode>().unwrap();
            let child_controller = cgroup_child.controller().lock();
            // This is race-free because if a child wants to activate a sub-controller, it should
            // first acquire the lock of the parent controller, which is held here.
            if child_controller.active_set().contains_type(ctrl_type) {
                return Err(Error::InvalidOperation);
            }
        }

        self.active_set.remove_type(ctrl_type);
        self.update_sub_controllers_for_descents(ctrl_type, current_node);

        Ok(())
    }

    fn update_sub_controllers_for_descents(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
    ) {
        fn update_sub_controller_for_one_child(
            child: &Arc<dyn SysObj>,
            ctrl_type: SubCtrlType,
            parent_controller: &LockedController,
        ) {
            let child_node = child.as_any().downcast_ref::<CgroupNode>().unwrap();
            match ctrl_type {
                SubCtrlType::Memory => {
                    let new_controller = SubController::new(Some(parent_controller));
                    child_node.controller().memory.update(new_controller);
                }
                SubCtrlType::CpuSet => {
                    let new_controller = SubController::new(Some(parent_controller));
                    child_node.controller().cpuset.update(new_controller);
                }
                SubCtrlType::Pids => {
                    let new_controller = SubController::new(Some(parent_controller));
                    child_node.controller().pids.update(new_controller);
                }
            }
        }

        let mut descents = VecDeque::new();

        // The following update logic is race-free due to the following reasons:
        //
        // 1. **No Concurrent Controller Activation/Deactivation**:
        //    At this point, we hold the controller lock for the current node and we know that the
        //    sub-controllers for the direct children are inactive. Then, no sub-controllers for
        //    any of the descendants can be activated before we release the lock.
        //
        // 2. **Concurrent Child Addition/Deletion is Fine**:
        //    We do need to consider that children may be added or removed concurrently. However,
        //    this is handled correctly:
        //    - If a child is added, it will attempt to hold its parent's controller lock, which is
        //      synchronized with the code below. If this happens after us, the up-to-date
        //      sub-controllers will be seen. If it happens before us, we will update the
        //      sub-controllers for it; due to race conditions, the sub-controllers may already be
        //      up to date, but updating them twice is harmless since they must not be activated.
        //    - If a child is removed, we may update a sub-controller that's about to be destroyed,
        //      which is harmless.

        // Update the direct children first.
        current_node.visit_children_with(0, &mut |child_node| {
            descents.push_back(child_node.clone());
            update_sub_controller_for_one_child(child_node, ctrl_type, self);

            Some(())
        });

        // Then update all the other descendent nodes.
        while let Some(node) = descents.pop_front() {
            let current_node = node.as_any().downcast_ref::<CgroupNode>().unwrap();
            // For descendent nodes, the sub-control must be inactive. But taking the controller
            // lock is necessary for synchronization purposes (see the explanation above).
            let locked_controller = current_node.controller().lock();
            current_node.visit_children_with(0, &mut |child_node| {
                descents.push_back(child_node.clone());
                update_sub_controller_for_one_child(child_node, ctrl_type, &locked_controller);

                Some(())
            });
        }
    }

    pub(super) fn active_set(&self) -> SubCtrlSet {
        *self.active_set
    }
}
