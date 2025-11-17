// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, string::String, sync::Arc, vec::Vec};

use aster_systree::{Error, Result, SysAttr, SysAttrSet, SysStr};
use bitflags::bitflags;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::{Mutex, MutexGuard, RcuOption},
    task::disable_preempt,
};

use crate::fs::cgroupfs::{
    controller::{cpuset::CpuSetController, memory::MemoryController, pids::PidsController},
    systree_node::CgroupSysNode,
    CgroupNode,
};

mod cpuset;
mod memory;
mod pids;

/// A trait to abstract all individual cgroup controllers.
trait SubControl {
    fn attr_set(&self) -> &SysAttrSet;

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize>;

    fn write_attr(
        &self,
        name: &str,
        reader: &mut VmReader,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize>;
}

/// An enum that wraps all possible cgroup sub-controller implementations.
//
// TODO: Currently uses an enum type instead of trait objects because RCU doesn't support
// `?Sized` objects in `Arc`. This may be changed to direct trait object usage in the future.
pub(super) enum SubController {
    Memory(MemoryController),
    CpuSet(CpuSetController),
    Pids(PidsController),
}

impl SubController {
    /// Creates an Arc-wrapped SubController if activation requirements are met.
    ///
    /// Activation logic for each controller:
    /// - `memory`: Always created, but may be inactive depending on `ctrl_state`.
    /// - `cpuset`: Created only if active in `ctrl_state`.
    /// - `pids`: Created only if active in `ctrl_state` and not root.
    fn new(name: &str, ctrl_state: SubCtrlState, is_root: bool) -> Option<Arc<Self>> {
        match name {
            "memory" => {
                let is_active = ctrl_state.contains(SubCtrlState::MEMORY_CTRLS);
                Some(Self::Memory(MemoryController::new(is_active, is_root)))
            }
            "cpuset" => {
                let is_active = ctrl_state.contains(SubCtrlState::CPUSET_CTRLS);
                is_active.then_some(Self::CpuSet(CpuSetController::new(is_root)))
            }
            "pids" => {
                let is_active = ctrl_state.contains(SubCtrlState::PIDS_CTRLS);
                (!is_root && is_active).then_some(Self::Pids(PidsController::new()))
            }
            _ => None,
        }
        .map(Arc::new)
    }

    fn as_subcontrol(&self) -> &dyn SubControl {
        match self {
            SubController::Memory(ctrl) => ctrl,
            SubController::CpuSet(ctrl) => ctrl,
            SubController::Pids(ctrl) => ctrl,
        }
    }

    fn attr_set(&self) -> &SysAttrSet {
        self.as_subcontrol().attr_set()
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        self.as_subcontrol()
            .read_attr_at(name, offset, writer, cgroup_node)
    }

    fn write_attr(
        &self,
        name: &str,
        reader: &mut VmReader,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        self.as_subcontrol().write_attr(name, reader, cgroup_node)
    }
}

bitflags! {
    /// Bitflags representing active/deactive sub-control state.
    pub(super) struct SubCtrlState: u8 {
        const MEMORY_CTRLS = 1 << 0;
        const CPUSET_CTRLS = 1 << 1;
        const PIDS_CTRLS = 1 << 2;
    }
}

impl SubCtrlState {
    pub(super) fn control_bit(name: &str) -> Option<Self> {
        match name {
            "memory" => Some(Self::MEMORY_CTRLS),
            "cpuset" => Some(Self::CPUSET_CTRLS),
            "pids" => Some(Self::PIDS_CTRLS),
            _ => None,
        }
    }

    /// Checks if a sub-control is active in the current state.
    ///
    /// If the given name does not represent a supported controller,
    /// returns `None`.
    pub(super) fn is_active(&self, name: &str) -> Option<bool> {
        Self::control_bit(name).map(|bit| self.contains(bit))
    }

    fn activate(&mut self, name: &str) {
        if let Some(bit) = Self::control_bit(name) {
            *self |= bit;
        }
    }

    fn deactivate(&mut self, name: &str) {
        if let Some(bit) = Self::control_bit(name) {
            *self -= bit;
        }
    }

    pub(super) fn show(&self) -> String {
        let mut controllers = Vec::new();

        if self.contains(Self::MEMORY_CTRLS) {
            controllers.push("memory");
        }
        if self.contains(Self::CPUSET_CTRLS) {
            controllers.push("cpuset");
        }
        if self.contains(Self::PIDS_CTRLS) {
            controllers.push("pids");
        }

        controllers.join(" ")
    }
}

/// The controller for a single cgroup.
///
/// This struct can manage the activation state of each sub-control, and dispatches read/write
/// operations to the appropriate sub-controllers.
///
/// The following is an explanation of the activation for sub-controls and controllers.
/// When a cgroup activates a specific sub-control (e.g., memory, io), it means this control
/// capability is being delegated to its children. Consequently, the corresponding controller
/// within the child nodes will be activated.
///
/// The root node serves as the origin for all these control capabilities, so the controllers
/// it possesses are always active. For any other node, only if its parent node first enables
/// a sub-control, its corresponding controller will be activated.
///
/// Among all nodes, the fundamental cgroup controller is always active.
pub(super) struct Controller {
    sub_ctrl_state: Mutex<SubCtrlState>,
    controllers: BTreeMap<SysStr, RcuOption<Arc<SubController>>>,
}

impl Controller {
    /// Creates a new controller manager for a cgroup.
    pub(super) fn new(ctrl_state: SubCtrlState, is_root: bool) -> Self {
        let mut controllers = BTreeMap::new();

        let memory_controller = SubController::new("memory", ctrl_state, is_root);
        controllers.insert(SysStr::from("memory"), RcuOption::new(memory_controller));
        let cpuset_controller = SubController::new("cpuset", ctrl_state, is_root);
        controllers.insert(SysStr::from("cpuset"), RcuOption::new(cpuset_controller));
        let pids_controller = SubController::new("pids", ctrl_state, is_root);
        controllers.insert(SysStr::from("pids"), RcuOption::new(pids_controller));

        Self {
            sub_ctrl_state: Mutex::new(SubCtrlState::empty()),
            controllers,
        }
    }

    pub(super) fn lock(&self) -> LockedController {
        LockedController {
            sub_ctrl_state: self.sub_ctrl_state.lock(),
            controller: self,
        }
    }

    /// Returns a string representation of the current `subtree_control` state.
    pub(super) fn show_state(&self) -> String {
        self.sub_ctrl_state.lock().show()
    }

    /// Returns a specific attribute with given name.
    pub(super) fn attr(&self, name: &str) -> Option<SysAttr> {
        let (subsys, _) = name.split_once('.')?;

        self.controllers
            .get(subsys)?
            .read()
            .get()?
            .attr_set()
            .get(name)
            .cloned()
    }

    pub(super) fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };

        let Some(rcu_controller) = self
            .controllers
            .get(subsys)
            .map(|controller| controller.read())
        else {
            return Err(Error::NotFound);
        };

        let Some(sub_controller) = rcu_controller.get() else {
            return Err(Error::NotFound);
        };

        sub_controller.read_attr_at(name, offset, writer, cgroup_node)
    }

    /// Iterates over all attributes in the active sub-controllers.
    ///
    /// Users can lock the controller of the parent cgroup node to ensure consistency.
    pub(super) fn for_each_attr<F>(&self, mut f: F)
    where
        F: FnMut(&SysAttr),
    {
        let guard = disable_preempt();

        for controller in self.controllers.values() {
            let rcu_controller = controller.read_with(&guard);
            if let Some(controller) = rcu_controller {
                for attr in controller.attr_set().iter() {
                    f(attr);
                }
            }
        }
    }
}

/// A locked controller for a cgroup.
///
/// Holding this lock indicates exclusive access to modify the sub-control state.
pub(super) struct LockedController<'a> {
    sub_ctrl_state: MutexGuard<'a, SubCtrlState>,
    controller: &'a Controller,
}

impl LockedController<'_> {
    /// Activates a sub-control with given name.
    pub(super) fn activate(
        &mut self,
        name: &str,
        current_node: &dyn CgroupSysNode,
        parent_controller: Option<&LockedController>,
    ) -> Result<()> {
        // Fast path: Invalid name or already activated.
        let Some(is_active) = self.sub_ctrl_state.is_active(name) else {
            return Err(Error::InvalidOperation);
        };
        if is_active {
            return Ok(());
        }

        // A cgroup can activate the sub-control only if this
        // sub-control has been activated in its parent cgroup.
        if parent_controller
            .is_some_and(|controller| !controller.sub_ctrl_state.is_active(name).unwrap())
        {
            return Err(Error::NotFound);
        }

        self.sub_ctrl_state.activate(name);
        current_node.visit_children_with(0, &mut |node| {
            let cgroup_node = node.as_any().downcast_ref::<CgroupNode>().unwrap();
            let rcu_controller = cgroup_node.controller().controllers.get(name).unwrap();
            rcu_controller.update(SubController::new(name, *self.sub_ctrl_state, false));

            Some(())
        });

        Ok(())
    }

    /// Deactivates a sub-control with given name.
    pub(super) fn deactivate(
        &mut self,
        name: &str,
        current_node: &dyn CgroupSysNode,
    ) -> Result<()> {
        // Fast path: Invalid name or already deactivated.
        let Some(is_active) = self.sub_ctrl_state.is_active(name) else {
            return Err(Error::InvalidOperation);
        };
        if !is_active {
            return Ok(());
        }

        // If any child node has activated this sub-control,
        // the deactivation operation will be rejected.
        let mut can_deactivate = true;
        current_node.visit_children_with(0, &mut |child| {
            let cgroup_child = child.as_any().downcast_ref::<CgroupNode>().unwrap();
            let child_controller = cgroup_child.controller().lock();
            if child_controller.sub_ctrl_state().is_active(name).unwrap() {
                can_deactivate = false;
                None
            } else {
                Some(())
            }
        });
        if !can_deactivate {
            return Err(Error::InvalidOperation);
        }

        self.sub_ctrl_state.deactivate(name);
        current_node.visit_children_with(0, &mut |node| {
            let cgroup_node = node.as_any().downcast_ref::<CgroupNode>().unwrap();
            let rcu_controller = cgroup_node.controller().controllers.get(name).unwrap();
            rcu_controller.update(SubController::new(name, *self.sub_ctrl_state, false));

            Some(())
        });

        Ok(())
    }

    pub(super) fn write_attr(
        &self,
        name: &str,
        reader: &mut VmReader,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };

        let Some(rcu_controller) = self
            .controller
            .controllers
            .get(subsys)
            .map(|controller| controller.read())
        else {
            return Err(Error::NotFound);
        };

        let Some(sub_controller) = rcu_controller.get() else {
            return Err(Error::NotFound);
        };

        sub_controller.write_attr(name, reader, cgroup_node)
    }

    pub(super) fn sub_ctrl_state(&self) -> SubCtrlState {
        *self.sub_ctrl_state
    }
}
