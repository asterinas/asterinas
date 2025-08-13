// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, string::String, sync::Arc, vec::Vec};

use aster_systree::{Error, Result, SysAttr, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysStr};
use bitflags::bitflags;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::{Mutex, MutexGuard, RcuOption},
    task::disable_preempt,
};

use crate::fs::cgroupfs::{
    controller::{
        cgroup::CgroupController, cpuset::CpuSetController, memory::MemoryController,
        pids::PidsController,
    },
    CgroupNode, CgroupSystem,
};

mod cgroup;
mod cpuset;
mod memory;
mod pids;

/// A trait to abstract all individual cgroup controllers.
trait SubControl {
    fn attr_set(&self) -> &SysAttrSet;

    fn read_attr(
        &self,
        name: &str,
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
// TODO: Currently uses an enum type instead of trait objects because RCU doesn't support
// `?Sized` objects in `Arc`. This may be changed to direct trait object usage in the future.
pub(super) enum SubController {
    Cgroup(CgroupController),
    Memory(MemoryController),
    CpuSet(CpuSetController),
    Pids(PidsController),
}

impl SubController {
    fn new(name: &str, ctrl_state: SubCtrlState, is_root: bool) -> Option<Arc<Self>> {
        match name {
            "cgroup" => Some(Self::Cgroup(CgroupController::new(is_root))),
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
            SubController::Cgroup(ctrl) => ctrl,
            SubController::Memory(ctrl) => ctrl,
            SubController::CpuSet(ctrl) => ctrl,
            SubController::Pids(ctrl) => ctrl,
        }
    }

    fn attr_set(&self) -> &SysAttrSet {
        self.as_subcontrol().attr_set()
    }

    fn read_attr(
        &self,
        name: &str,
        writer: &mut VmWriter,
        cgroup_node: &dyn CgroupSysNode,
    ) -> Result<usize> {
        self.as_subcontrol().read_attr(name, writer, cgroup_node)
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
    /// Bitflags representing enabled/disabled sub-control state.
    pub(super) struct SubCtrlState: u8 {
        const MEMORY_CTRLS = 1 << 0;
        const CPUSET_CTRLS = 1 << 1;
        const PIDS_CTRLS = 1 << 2;
    }
}

impl SubCtrlState {
    fn control_bit(name: &str) -> Option<Self> {
        match name {
            "memory" => Some(Self::MEMORY_CTRLS),
            "cpuset" => Some(Self::CPUSET_CTRLS),
            "pids" => Some(Self::PIDS_CTRLS),
            _ => None,
        }
    }

    /// Checks if a sub-control is enabled in the current state.
    ///
    /// If the given name does not represent a supported controller,
    /// returns `None`.
    fn is_enabled(&self, name: &str) -> Option<bool> {
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

    fn show(&self) -> String {
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

/// The main controller for a single cgroup.
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
    /// All attributes within the current controller.
    ///
    /// This field must be updated whenever the state of active controllers changes.
    all_attrs: Mutex<SysAttrSet>,
}

impl Controller {
    /// Creates a new controller manager for a cgroup.
    pub(super) fn new(ctrl_state: SubCtrlState, is_root: bool) -> Self {
        let mut controllers = BTreeMap::new();

        let cgroup_controller = SubController::new("cgroup", ctrl_state, is_root).unwrap();
        controllers.insert(
            SysStr::from("cgroup"),
            RcuOption::new(Some(cgroup_controller)),
        );

        let memory_controller = SubController::new("memory", ctrl_state, is_root);
        controllers.insert(SysStr::from("memory"), RcuOption::new(memory_controller));
        let cpuset_controller = SubController::new("cpuset", ctrl_state, is_root);
        controllers.insert(SysStr::from("cpuset"), RcuOption::new(cpuset_controller));
        let pids_controller = SubController::new("pids", ctrl_state, is_root);
        controllers.insert(SysStr::from("pids"), RcuOption::new(pids_controller));

        let controller = Self {
            sub_ctrl_state: Mutex::new(SubCtrlState::empty()),
            controllers,
            all_attrs: Mutex::new(SysAttrSet::new_empty()),
        };
        controller.update_attrs();

        controller
    }

    pub(super) fn sub_ctrl_state(&self) -> MutexGuard<SubCtrlState> {
        self.sub_ctrl_state.lock()
    }

    /// Returns a string representation of the current `subtree_control` state.
    pub(super) fn show_state(&self) -> String {
        self.sub_ctrl_state.lock().show()
    }

    /// Returns a specific attribute with given name.
    pub(super) fn attr(&self, name: &str) -> Option<SysAttr> {
        self.all_attrs.lock().get(name).cloned()
    }

    /// Returns a the entire attribute set of this node.
    pub(super) fn node_attrs(&self) -> SysAttrSet {
        self.all_attrs.lock().clone()
    }

    /// Rebuilds the `all_attrs` set.
    ///
    /// This should be called whenever the state of active controllers changes.
    fn update_attrs(&self) {
        let mut builder = SysAttrSetBuilder::new();
        let guard = disable_preempt();
        for controller in self.controllers.values() {
            let rcu_controller = controller.read_with(&guard);
            if let Some(controller) = rcu_controller {
                for attr in controller.attr_set().iter() {
                    builder.add(attr.name().clone(), attr.perms());
                }
            }
        }

        *self.all_attrs.lock() = builder.build().unwrap();
    }

    /// Activates a sub-control with given name.
    pub(super) fn activate(&self, name: &str, current_node: &dyn CgroupSysNode) -> Result<()> {
        let mut sub_ctrl_state = self.sub_ctrl_state.lock();
        let Some(is_enabled) = sub_ctrl_state.is_enabled(name) else {
            return Err(Error::InvalidOperation);
        };

        if is_enabled {
            return Ok(());
        }

        sub_ctrl_state.activate(name);

        current_node.visit_children_with(0, &mut |node| {
            let cgroup_node = node.as_any().downcast_ref::<CgroupNode>().unwrap();
            let rcu_controller = cgroup_node.controller().controllers.get(name).unwrap();
            rcu_controller.update(SubController::new(name, *sub_ctrl_state, false));

            cgroup_node.controller().update_attrs();

            Some(())
        });

        Ok(())
    }

    /// Deactivates a sub-control with given name.
    pub(super) fn deactivate(&self, name: &str, current_node: &dyn CgroupSysNode) -> Result<()> {
        let mut sub_ctrl_state = self.sub_ctrl_state.lock();
        let Some(is_enabled) = sub_ctrl_state.is_enabled(name) else {
            return Err(Error::InvalidOperation);
        };

        if !is_enabled {
            return Ok(());
        }

        sub_ctrl_state.deactivate(name);

        current_node.visit_children_with(0, &mut |node| {
            let cgroup_node = node.as_any().downcast_ref::<CgroupNode>().unwrap();
            let rcu_controller = cgroup_node.controller().controllers.get(name).unwrap();
            rcu_controller.update(SubController::new(name, *sub_ctrl_state, false));

            cgroup_node.controller().update_attrs();

            Some(())
        });

        Ok(())
    }

    pub(super) fn read_attr(
        &self,
        name: &str,
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

        let Some(controller) = rcu_controller.get() else {
            return Err(Error::NotFound);
        };

        controller.read_attr(name, writer, cgroup_node)
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
            .controllers
            .get(subsys)
            .map(|controller| controller.read())
        else {
            return Err(Error::NotFound);
        };

        let Some(controller) = rcu_controller.get() else {
            return Err(Error::NotFound);
        };

        controller.write_attr(name, reader, cgroup_node)
    }
}

/// A trait that abstracts over different types of cgroup nodes (`CgroupNode`, `CgroupSystem`)
/// to provide a common API for controller logics.
pub(super) trait CgroupSysNode: SysBranchNode {
    fn controller(&self) -> &Controller;

    fn cgroup_parent(&self) -> Option<Arc<dyn CgroupSysNode>> {
        let parent = self.parent()?;
        if parent.is_root() {
            Some(Arc::downcast::<CgroupSystem>(parent).unwrap())
        } else {
            Some(Arc::downcast::<CgroupNode>(parent).unwrap())
        }
    }
}
