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
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttr, SysAttrSet,
    SysAttrSetBuilder, SysBranchNode, SysObj, SysPerms, SysStr, MAX_ATTR_SIZE,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

use crate::{
    fs::cgroupfs::controller::{Controller, SubCtrlState},
    prelude::*,
    process::{process_table, signal::constants::SIGSTOP, Pid, Process},
};

/// A type that provides exclusive, synchronized access to modify cgroup membership.
///
/// This struct encapsulates the logic for moving processes between cgroups.
/// By calling `CgroupMembership::lock()`, a thread can attempt to acquire a lock
/// on the global instance. Upon success, it returns a guard that provides mutable
/// access, allowing for safe cgroup membership modifications.
///
/// # Usage
///
/// ```rust,ignore
/// // Acquire the lock.
/// let membership = CgroupMembership::lock();
///
/// // Move a process to a new cgroup node.
/// membership.move_process_to_node(process, &new_cgroup);
///
/// // The lock is automatically released when `membership` is dropped.
/// ```
pub struct CgroupMembership {
    _private: (),
}

impl CgroupMembership {
    /// Acquires the lock on the global instance.
    ///
    /// Returns a guard that provides mutable access to modify cgroup membership.
    pub fn lock() -> MutexGuard<'static, Self> {
        static CGROUP_MEMBERSHIP: Mutex<CgroupMembership> =
            Mutex::new(CgroupMembership { _private: () });

        CGROUP_MEMBERSHIP.lock()
    }

    /// Moves a process to the new cgroup node.
    ///
    /// A process can only belong to one cgroup at a time.
    /// When moved to a new cgroup, it's automatically removed from the
    /// previous one.
    pub fn move_process_to_node(
        &mut self,
        process: Arc<Process>,
        new_cgroup: &CgroupNode,
    ) -> Result<()> {
        if let Some(old_cgroup) = process.cgroup().get() {
            // Fast path: If the process is already in this cgroup, do nothing.
            if new_cgroup.id() == old_cgroup.id() {
                return Ok(());
            }

            old_cgroup
                .with_inner_mut(|inner| {
                    inner.processes.remove(&process.pid()).unwrap();
                    if inner.processes.is_empty() {
                        let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                        if old_count == 1 {
                            old_cgroup.propagate_sub_populated();
                        }
                    }
                })
                .unwrap();
        };

        new_cgroup
            .with_inner_mut(|inner| {
                if inner.processes.is_empty() {
                    let old_count = new_cgroup.populated_count.fetch_add(1, Ordering::Relaxed);
                    if old_count == 0 {
                        new_cgroup.propagate_add_populated();
                    }
                }
                inner
                    .processes
                    .insert(process.pid(), Arc::downgrade(&process));
            })
            .ok_or(Error::IsDead)?;

        process.set_cgroup(Some(new_cgroup.fields.weak_self().upgrade().unwrap()));

        Ok(())
    }

    /// Moves a process to the root cgroup.
    pub fn move_process_to_root(&mut self, process: &Process) {
        let process_cgroup = process.cgroup();
        let Some(old_cgroup) = process_cgroup.get() else {
            return;
        };

        old_cgroup
            .with_inner_mut(|inner| {
                inner.processes.remove(&process.pid()).unwrap();
                if inner.processes.is_empty() {
                    let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                    if old_count == 1 {
                        old_cgroup.propagate_sub_populated();
                    }
                }
            })
            .unwrap();

        process.set_cgroup(None);
    }

    fn freeze_cgroup_node(&mut self, cgroup_node: &CgroupNode, freeze_op: FreezeOp) -> Result<()> {
        let mut worklist: Vec<Arc<CgroupNode>> =
            vec![cgroup_node.fields.weak_self().upgrade().unwrap()];
        while let Some(node) = worklist.pop() {
            let Some(has_changed) = node.with_inner_mut(|inner| inner.do_freeze(freeze_op)) else {
                if node.depth == cgroup_node.depth {
                    return Err(Error::IsDead);
                }
                continue;
            };

            if has_changed {
                for child in node.fields.children_ref().read().values() {
                    worklist.push(child.clone());
                }
            }

            // TODO: Add the logics of upward propagation.
        }

        Ok(())
    }
}

/// The root of a cgroup hierarchy, serving as the entry point to
/// the entire cgroup control system.
///
/// The cgroup system provides v2 unified hierarchy, and is also used as a root
/// node in the cgroup systree.
pub(super) struct CgroupSystem {
    fields: BranchNodeFields<CgroupNode, Self>,
    controller: Controller,
}

impl Debug for CgroupSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupSystem")
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

/// A control group node in the cgroup systree.
///
/// Each node can bind a group of processes together for purpose of resource
/// management. Except for the root node, all nodes in the cgroup tree are of
/// this type.
pub struct CgroupNode {
    fields: BranchNodeFields<CgroupNode, Self>,
    /// The controller of this cgroup node.
    controller: Controller,
    /// The inner data. If it is `None`, then the cgroup node is dead.
    inner: RwMutex<Option<Inner>>,
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
        f.debug_struct("CgroupNode")
            .field("fields", &self.fields)
            .field("populated_count", &self.populated_count)
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Inner {
    /// Processes bound to the cgroup node.
    processes: BTreeMap<Pid, Weak<Process>>,
    /// Whether the cgroup node is frozen.
    is_frozen: bool,
}

impl Inner {
    fn new(is_frozen: bool) -> Self {
        Self {
            processes: BTreeMap::new(),
            is_frozen,
        }
    }

    fn do_freeze(&mut self, freeze_op: FreezeOp) -> bool {
        match freeze_op {
            FreezeOp::Freeze => {
                if self.is_frozen {
                    return false;
                }

                for weak_process in self.processes.values() {
                    if let Some(process) = weak_process.upgrade() {
                        process.stop(SIGSTOP);
                    }
                }

                self.is_frozen = true;
                true
            }
            FreezeOp::Unfreeze => {
                if !self.is_frozen {
                    return false;
                }

                for weak_process in self.processes.values() {
                    if let Some(process) = weak_process.upgrade() {
                        process.resume();
                    }
                }
                self.is_frozen = false;
                true
            }
        }
    }
}

#[inherit_methods(from = "self.fields")]
impl CgroupSystem {
    /// Adds a child node.
    fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

#[inherit_methods(from = "self.fields")]
impl CgroupNode {
    /// Adds a child node.
    fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

impl CgroupSystem {
    /// Returns the `CgroupSystem` singleton.
    pub(super) fn singleton() -> &'static Arc<CgroupSystem> {
        static SINGLETON: Once<Arc<CgroupSystem>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.subtree_control"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.procs"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
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
    pub(self) fn new(
        name: SysStr,
        depth: usize,
        sub_ctrl_state: SubCtrlState,
        is_frozen: bool,
    ) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.subtree_control"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.procs"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.events"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.freeze"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            CgroupNode {
                fields,
                controller: Controller::new(sub_ctrl_state, false),
                inner: RwMutex::new(Some(Inner::new(is_frozen))),
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
    fn propagate_add_populated(&self) {
        if self.depth <= 1 {
            return;
        }

        let mut current_parent = Arc::downcast::<CgroupNode>(self.parent().unwrap()).unwrap();
        loop {
            let old_count = current_parent
                .populated_count
                .fetch_add(1, Ordering::Relaxed);
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
                .fetch_sub(1, Ordering::Relaxed);
            if old_count != 1 {
                break;
            }

            if current_parent.depth == 1 {
                break;
            }

            current_parent = Arc::downcast::<CgroupNode>(current_parent.parent().unwrap()).unwrap();
        }
    }

    /// Performs a read-only operation on the inner data.
    ///
    /// If the cgroup node is dead, returns `None`.
    #[must_use]
    fn with_inner<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&Inner) -> R,
    {
        let inner = self.inner.read();
        let inner_ref = inner.as_ref()?;

        Some(op(inner_ref))
    }

    /// Performs a mutable operation on the inner data.
    ///
    /// If the cgroup node is dead, returns `None`.
    #[must_use]
    fn with_inner_mut<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&mut Inner) -> R,
    {
        let mut inner = self.inner.write();
        let inner_ref = inner.as_mut()?;

        Some(op(inner_ref))
    }

    /// Marks this cgroup node as dead.
    ///
    /// This will succeed only if the cgroup node is empty and is alive.
    /// Here, a cgroup node is considered empty if it has no child nodes and no
    /// processes bound to it.
    pub(super) fn mark_as_dead(&self) -> crate::prelude::Result<()> {
        let mut inner = self.inner.write();
        let Some(inner_ref) = inner.as_ref() else {
            return_errno_with_message!(Errno::ENOENT, "the cgroup node is already dead");
        };

        if !inner_ref.processes.is_empty() {
            return_errno_with_message!(Errno::EBUSY, "the cgroup hierarchy still has processes");
        }

        let children = self.fields.children_ref().read();
        if !children.is_empty() {
            return_errno_with_message!(
                Errno::ENOTEMPTY,
                "only an empty cgroup hierarchy can be removed"
            );
        }

        *inner = None;

        Ok(())
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
        if name.starts_with("cgroup.") {
            self.fields.attr_set().get(name).cloned()
        } else {
            self.controller.attr(name)
        }
    }

    fn node_attrs(&self) -> Cow<SysAttrSet> {
        let mut builder = SysAttrSetBuilder::new();
        for attr in self.fields.attr_set().iter() {
            builder.add(attr.name().clone(), attr.perms());
        }

        self.controller.for_each_attr(|attr| {
            builder.add(attr.name().clone(), attr.perms());
        });

        Cow::Owned(builder.build().unwrap())
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "cgroup.procs" => {
                let process_table = process_table::process_table_mut();
                for process in process_table.iter() {
                    if process.cgroup().is_none() {
                        writeln!(printer, "{}", process.pid())?;
                    }
                }
            }
            "cgroup.controllers" => {
                writeln!(printer, "{}", SubCtrlState::all().show())?;
            }
            "cgroup.subtree_control" => {
                let context = self.controller.show_state();
                writeln!(printer, "{}", context)?;
            }
            // TODO: Add support for reading other attributes.
            _ => return self.controller.read_attr_at(name, offset, writer, self),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let pid = content
                    .to_str()
                    .ok()
                    .and_then(|string| string.trim().parse::<Pid>().ok())
                    .ok_or(Error::InvalidOperation)?;

                with_process_cgroup_locked(pid, |process, cgroup_membership| {
                    cgroup_membership.move_process_to_root(&process);
                    Ok(())
                })?;

                Ok(len)
            }
            "cgroup.subtree_control" => {
                let (actions, len) = read_subtree_control_from_reader(reader)?;

                // The Lock order: current controller -> child controllers
                let mut controller = self.controller.lock();
                for action in actions {
                    match action {
                        SubControlAction::Activate(name) => {
                            controller.activate(&name, self, None)?;
                        }
                        SubControlAction::Deactivate(name) => {
                            controller.deactivate(&name, self)?;
                        }
                    }
                }

                Ok(len)
            }
            // TODO: Add support for writing other attributes.
            _ => self.controller.lock().write_attr(name, reader, self),
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let controller = self.controller.lock();
        let new_child = CgroupNode::new(
            name.to_string().into(),
            1,
            controller.sub_ctrl_state(),
            false,
        );
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});

inherit_sys_branch_node!(CgroupNode, fields, {
    fn attr(&self, name: &str) -> Option<SysAttr> {
        if name.starts_with("cgroup.") {
            self.fields.attr_set().get(name).cloned()
        } else {
            self.controller.attr(name)
        }
    }

    fn node_attrs(&self) -> Cow<SysAttrSet> {
        let mut builder = SysAttrSetBuilder::new();
        for attr in self.fields.attr_set().iter() {
            builder.add(attr.name().clone(), attr.perms());
        }

        let parent_node = self.cgroup_parent().unwrap();
        let _controller_guard = parent_node.controller().lock();

        self.controller.for_each_attr(|attr| {
            builder.add(attr.name().clone(), attr.perms());
        });

        Cow::Owned(builder.build().unwrap())
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.with_inner(|inner| {
            let mut printer = VmPrinter::new_skip(writer, offset);
            match name {
                "cgroup.procs" => {
                    for pid in inner.processes.keys() {
                        writeln!(printer, "{}", pid)?;
                    }
                }
                "cgroup.events" => {
                    let res = if self.populated_count.load(Ordering::Relaxed) > 0 {
                        1
                    } else {
                        0
                    };

                    writeln!(printer, "populated {}", res)?;
                    writeln!(printer, "frozen {}", if inner.is_frozen { 1 } else { 0 })?;
                }
                "cgroup.controllers" => {
                    let context = self.cgroup_parent().unwrap().controller().show_state();
                    writeln!(printer, "{}", context)?;
                }
                "cgroup.subtree_control" => {
                    let context = self.controller.show_state();
                    writeln!(printer, "{}", context)?;
                }
                "cgroup.freeze" => {
                    let res = if inner.is_frozen { 1 } else { 0 };
                    writeln!(printer, "{}", res)?;
                }
                // TODO: Add support for reading other attributes.
                _ => {
                    return self.controller.read_attr_at(name, offset, writer, self);
                }
            }

            Ok(printer.bytes_written())
        })
        .ok_or(Error::IsDead)?
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let pid = content
                    .to_str()
                    .ok()
                    .and_then(|string| string.trim().parse::<Pid>().ok())
                    .ok_or(Error::InvalidOperation)?;

                let controller = self.controller.lock();
                // According to "no internal processes" rule of cgroupv2, if a non-root
                // cgroup node has activated some sub-controls, it cannot bind any process.
                //
                // Ref: https://man7.org/linux/man-pages/man7/cgroups.7.html
                if !controller.sub_ctrl_state().is_empty() {
                    return Err(Error::ResourceUnavailable);
                }

                with_process_cgroup_locked(pid, |target_process, cgroup_membership| {
                    // TODO: According to the "no internal processes" rule of cgroupv2
                    // (Ref: https://man7.org/linux/man-pages/man7/cgroups.7.html),
                    // if the cgroup node has enabled some controllers like "memory", "io",
                    // it is forbidden to bind a process to an internal cgroup node.
                    cgroup_membership.move_process_to_node(target_process, self)
                })?;

                Ok(len)
            }
            "cgroup.subtree_control" => {
                let (actions, len) = read_subtree_control_from_reader(reader)?;

                // The Lock order: parent controller -> current controller -> child controllers
                let parent_node = self.cgroup_parent().unwrap();
                let parent_controller = parent_node.controller().lock();
                let mut current_controller = self.controller.lock();

                self.with_inner(|inner| {
                    // According to "no internal processes" rule of cgroupv2, if a non-root
                    // cgroup node has bound processes, it cannot activate any sub-control.
                    //
                    // Ref: https://man7.org/linux/man-pages/man7/cgroups.7.html
                    if !inner.processes.is_empty() {
                        return Err(Error::ResourceUnavailable);
                    }

                    for action in actions {
                        match action {
                            SubControlAction::Activate(name) => {
                                current_controller.activate(
                                    &name,
                                    self,
                                    Some(&parent_controller),
                                )?;
                            }
                            SubControlAction::Deactivate(name) => {
                                current_controller.deactivate(&name, self)?;
                            }
                        }
                    }

                    Ok(len)
                })
                .ok_or(Error::IsDead)?
            }
            "cgroup.freeze" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let freeze_op = content
                    .to_str()
                    .map_err(|_| Error::InvalidOperation)?
                    .trim();
                let freeze_op = match freeze_op {
                    "0" => FreezeOp::Unfreeze,
                    "1" => FreezeOp::Freeze,
                    _ => return Err(Error::InvalidOperation),
                };
                CgroupMembership::lock().freeze_cgroup_node(self, freeze_op)?;

                Ok(len)
            }
            // TODO: Add support for writing other attributes.
            _ => {
                let controller = self.controller.lock();
                self.with_inner(|_| controller.write_attr(name, reader, self))
                    .ok_or(Error::IsDead)?
            }
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let controller = self.controller.lock();
        self.with_inner(|inner| {
            let new_child = CgroupNode::new(
                name.to_string().into(),
                self.depth + 1,
                controller.sub_ctrl_state(),
                inner.is_frozen,
            );
            self.add_child(new_child.clone())?;
            Ok(new_child as _)
        })
        // TODO: This should be checked at upper layers.
        .ok_or(Error::NotFound)?
    }
});

/// A helper function to safely perform an operation on a process's cgroup.
///
/// The given `pid` means the PID of the target process. A PID of 0 refers to the
/// current process.
///
/// Returns `Error::InvalidOperation` if the PID is not found or if the target
/// process is a zombie.
pub(super) fn with_process_cgroup_locked<F>(pid: Pid, op: F) -> Result<()>
where
    F: FnOnce(Arc<Process>, &mut CgroupMembership) -> Result<()>,
{
    let process = if pid == 0 {
        current!()
    } else {
        process_table::get_process(pid).ok_or(Error::InvalidOperation)?
    };

    let mut cgroup_guard = CgroupMembership::lock();
    if process.status().is_zombie() {
        return Err(Error::InvalidOperation);
    }

    op(process, &mut cgroup_guard)
}

enum SubControlAction {
    Activate(String),
    Deactivate(String),
}

/// Reads the actions for sub-control from the given reader.
///
/// Returns a tuple containing vector of actions and the number of bytes read.
fn read_subtree_control_from_reader(
    reader: &mut VmReader,
) -> Result<(Vec<SubControlAction>, usize)> {
    let (content, len) = reader
        .read_cstring_until_end(MAX_ATTR_SIZE)
        .map_err(|_| Error::PageFault)?;
    let context = content.to_str().map_err(|_| Error::InvalidOperation)?;

    let mut actions_vec = Vec::new();
    let actions = context.split_whitespace();
    for action in actions {
        if action.len() < 2 {
            return Err(Error::InvalidOperation);
        }

        let action = match action.chars().next() {
            Some('+') => {
                let name = action[1..].to_string();
                if SubCtrlState::control_bit(&name).is_none() {
                    return Err(Error::InvalidOperation);
                }

                SubControlAction::Activate(name)
            }
            Some('-') => {
                let name = action[1..].to_string();
                if SubCtrlState::control_bit(&name).is_none() {
                    return Err(Error::InvalidOperation);
                }

                SubControlAction::Deactivate(name)
            }
            _ => return Err(Error::InvalidOperation),
        };
        actions_vec.push(action);
    }

    Ok((actions_vec, len))
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

#[derive(Copy, Clone)]
enum FreezeOp {
    Freeze,
    Unfreeze,
}
