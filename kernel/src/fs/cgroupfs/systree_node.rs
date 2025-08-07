// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    string::ToString,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_systree::{
    inherit_sys_branch_node, AttrLessBranchNodeFields, Result, SysAttr, SysAttrSet, SysBranchNode,
    SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use crate::{
    fs::cgroupfs::controller::{CgroupSysNode, Controller, SubCtrlState},
    prelude::*,
    process::{Pid, Process},
};

/// The root of a cgroup hierarchy, serving as the entry point to
/// the entire cgroup control system.
///
/// The cgroup system provides v2 unified hierarchy, and is also used as a root
/// node in the cgroup systree.
pub struct CgroupSystem {
    fields: AttrLessBranchNodeFields<CgroupNode, Self>,
    controller: Controller,
}

impl Debug for CgroupSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupSystem")
            .field("fields", &self.fields)
            .finish()
    }
}

/// A control group node in the cgroup systree.
///
/// Each node can bind a group of processes together for purpose of resource
/// management. Except for the root node, all nodes in the cgroup tree are of
/// this type.
pub struct CgroupNode {
    fields: AttrLessBranchNodeFields<CgroupNode, Self>,
    /// The controller of this cgroup node.
    controller: Controller,
    /// Processes bound to this node.
    processes: Mutex<BTreeMap<Pid, Weak<Process>>>,
    /// The depth of the node in the cgroupfs [`SysTree`], where the child of
    /// the root node has a depth of 1.
    depth: usize,
    /// Tracks the "populated" status of this node and its direct children.
    ///
    /// The count is the sum of:
    /// - The number of its direct children that are populated.
    /// - A value of 1 if this node itself contains processes.
    ///
    /// "populated": A node is considered populated if it has bound processes
    /// either on itself or in any of its descendant nodes. Consequently,
    /// a count > 0 indicates that this node is populated.
    populated_count: AtomicUsize,
}

impl Debug for CgroupNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupNormalNode")
            .field("fields", &self.fields)
            .finish()
    }
}

#[inherit_methods(from = "self.fields")]
impl CgroupSystem {
    /// Adds a child node.
    pub fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

#[inherit_methods(from = "self.fields")]
impl CgroupNode {
    /// Adds a child node.
    pub fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

impl CgroupSystem {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            CgroupSystem {
                fields,
                controller: Controller::new(SubCtrlState::all(), true),
            }
        })
    }
}

impl CgroupSysNode for CgroupSystem {
    fn controller(&self) -> &Controller {
        &self.controller
    }
}

impl CgroupNode {
    pub(super) fn new(name: SysStr, depth: usize, sub_ctrl_state: SubCtrlState) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            CgroupNode {
                fields,
                controller: Controller::new(sub_ctrl_state, false),
                processes: Mutex::new(BTreeMap::new()),
                depth,
                populated_count: AtomicUsize::new(0),
            }
        })
    }
}

impl CgroupSysNode for CgroupNode {
    fn controller(&self) -> &Controller {
        &self.controller
    }
}

// For process management
impl CgroupNode {
    /// Moves a process to this cgroup node.
    ///
    /// A process can only belong to one cgroup at a time.
    /// When moved to a new cgroup, it's automatically removed
    /// from the previous one.
    pub fn move_process(&self, process: Arc<Process>) {
        let rcu_old_cgroup = process.cgroup();
        let old_cgroup = rcu_old_cgroup.get();

        let (mut current_process_set, old_cgroup_process_set) = {
            if let Some(old_cgroup) = old_cgroup.as_ref() {
                if self.id() == old_cgroup.id() {
                    return;
                }

                if self.id() < old_cgroup.id() {
                    let current_process_set = self.processes.lock();
                    let old_cgroup_process_set = old_cgroup.processes.lock();
                    (current_process_set, Some(old_cgroup_process_set))
                } else {
                    let old_cgroup_process_set = old_cgroup.processes.lock();
                    let current_process_set = self.processes.lock();
                    (current_process_set, Some(old_cgroup_process_set))
                }
            } else {
                (self.processes.lock(), None)
            }
        };

        if let Some(mut old_cgroup_process_set) = old_cgroup_process_set {
            if old_cgroup_process_set.remove(&process.pid()).is_some()
                && old_cgroup_process_set.is_empty()
            {
                let old_count = self.populated_count.fetch_sub(1, Ordering::AcqRel);
                if old_count == 1 {
                    self.propagate_sub_populated();
                }
            }
        }

        process.set_cgroup(Some(self.fields.weak_self().upgrade().unwrap()));

        if current_process_set.is_empty() {
            let old_count = self.populated_count.fetch_add(1, Ordering::AcqRel);
            if old_count == 0 {
                self.propagate_add_populated();
            }
        }

        current_process_set.insert(process.pid(), Arc::downgrade(&process));
    }

    /// Removes a process from this cgroup node.
    pub fn remove_process(&self, process: &Process) {
        let mut processes = self.processes.lock();
        if processes.remove(&process.pid()).is_none() {
            return;
        }

        process.cgroup().compare_exchange(None).unwrap();

        if processes.is_empty() {
            let old_count = self.populated_count.fetch_sub(1, Ordering::AcqRel);
            if old_count == 1 {
                self.propagate_sub_populated();
            }
        }
    }

    fn propagate_add_populated(&self) {
        if self.depth <= 1 {
            return;
        }

        let mut current_parent = Arc::downcast::<CgroupNode>(self.parent().unwrap()).unwrap();
        loop {
            let old_count = current_parent
                .populated_count
                .fetch_add(1, Ordering::AcqRel);
            if old_count > 0 {
                break;
            }

            if current_parent.depth == 1 {
                break;
            }

            current_parent = Arc::downcast::<CgroupNode>(current_parent.parent().unwrap()).unwrap();
        }
    }

    fn propagate_sub_populated(&self) {
        if self.depth <= 1 {
            return;
        }

        let mut current_parent = Arc::downcast::<CgroupNode>(self.parent().unwrap()).unwrap();
        loop {
            let old_count = current_parent
                .populated_count
                .fetch_sub(1, Ordering::AcqRel);
            if old_count != 1 {
                break;
            }

            if current_parent.depth == 1 {
                break;
            }

            current_parent = Arc::downcast::<CgroupNode>(current_parent.parent().unwrap()).unwrap();
        }
    }

    /// Whether this cgroup node has any processes bound to it.
    pub fn have_processes(&self) -> bool {
        !self.processes.lock().is_empty()
    }

    /// Reads the PID of the processes bound to this cgroup node.
    pub(super) fn read_procs(&self) -> String {
        self.processes
            .lock()
            .keys()
            .map(|pid| pid.to_string())
            .collect::<Vec<String>>()
            .join("\n")
    }

    pub(super) fn populated_count(&self) -> &AtomicUsize {
        &self.populated_count
    }
}

inherit_sys_branch_node!(CgroupSystem, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {
        // This method should be a no-op for `RootNode`.
    }

    fn attr(&self, name: &str) -> Option<SysAttr> {
        self.controller.attr(name)
    }

    fn node_attrs(&self) -> Cow<SysAttrSet> {
        Cow::Owned(self.controller.node_attrs())
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.controller.read_attr(name, writer, self)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        self.controller.write_attr(name, reader, self)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let sub_ctrl_state = self.controller().sub_ctrl_state();
        let new_child = CgroupNode::new(name.to_string().into(), 1, *sub_ctrl_state);
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});

inherit_sys_branch_node!(CgroupNode, fields, {
    fn attr(&self, name: &str) -> Option<SysAttr> {
        self.controller.attr(name)
    }

    fn node_attrs(&self) -> Cow<SysAttrSet> {
        Cow::Owned(self.controller.node_attrs())
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.controller.read_attr(name, writer, self)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        self.controller.write_attr(name, reader, self)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let sub_ctrl_state = self.controller().sub_ctrl_state();
        let new_child = CgroupNode::new(name.to_string().into(), self.depth + 1, *sub_ctrl_state);
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});
