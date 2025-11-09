// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode,
    SysObj, SysPerms, SysStr, MAX_ATTR_SIZE,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

use crate::{
    prelude::*,
    process::{process_table, Pid, Process},
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
                .with_inner_mut(|old_cgroup_processes| {
                    old_cgroup_processes.remove(&process.pid()).unwrap();
                    if old_cgroup_processes.is_empty() {
                        let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                        if old_count == 1 {
                            old_cgroup.propagate_sub_populated();
                        }
                    }
                })
                .unwrap();
        };

        new_cgroup
            .with_inner_mut(|current_processes| {
                if current_processes.is_empty() {
                    let old_count = new_cgroup.populated_count.fetch_add(1, Ordering::Relaxed);
                    if old_count == 0 {
                        new_cgroup.propagate_add_populated();
                    }
                }
                current_processes.insert(process.pid(), Arc::downgrade(&process));
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
            .with_inner_mut(|old_cgroup_processes| {
                old_cgroup_processes.remove(&process.pid()).unwrap();
                if old_cgroup_processes.is_empty() {
                    let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                    if old_count == 1 {
                        old_cgroup.propagate_sub_populated();
                    }
                }
            })
            .unwrap();

        process.set_cgroup(None);
    }
}

/// The root of a cgroup hierarchy, serving as the entry point to
/// the entire cgroup control system.
///
/// The cgroup system provides v2 unified hierarchy, and is also used as a root
/// node in the cgroup systree.
#[derive(Debug)]
pub(super) struct CgroupSystem {
    fields: BranchNodeFields<CgroupNode, Self>,
}

/// A control group node in the cgroup systree.
///
/// Each node can bind a group of processes together for purpose of resource
/// management. Except for the root node, all nodes in the cgroup tree are of
/// this type.
pub struct CgroupNode {
    fields: BranchNodeFields<CgroupNode, Self>,
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
            SysStr::from("cpu.pressure"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(SysStr::from("cpu.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            CgroupSystem { fields }
        })
    }
}

impl CgroupNode {
    pub(self) fn new(name: SysStr, depth: usize) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
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
            SysStr::from("cpu.pressure"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(SysStr::from("cpu.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(
            SysStr::from("cgroup.events"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            CgroupNode {
                fields,
                inner: RwMutex::new(Some(Inner::default())),
                depth,
                populated_count: AtomicUsize::new(0),
            }
        })
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
    fn with_inner<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&BTreeMap<Pid, Weak<Process>>) -> R,
    {
        let inner = self.inner.read();
        let inner_ref = inner.as_ref()?;

        Some(op(&inner_ref.processes))
    }

    /// Performs a mutable operation on the inner data.
    ///
    /// If the cgroup node is dead, returns `None`.
    fn with_inner_mut<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&mut BTreeMap<Pid, Weak<Process>>) -> R,
    {
        let mut inner = self.inner.write();
        let inner_ref = inner.as_mut()?;

        Some(op(&mut inner_ref.processes))
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
            _ => {
                // TODO: Add support for reading other attributes.
                return Err(Error::AttributeError);
            }
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
            _ => {
                // TODO: Add support for writing other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let new_child = CgroupNode::new(name.to_string().into(), 1);
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});

inherit_sys_branch_node!(CgroupNode, fields, {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.with_inner(|processes| {
            let mut printer = VmPrinter::new_skip(writer, offset);
            match name {
                "cgroup.procs" => {
                    for pid in processes.keys() {
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
                    // Currently we have not enabled the "frozen" attribute
                    // so the "frozen" field is always zero.
                    writeln!(printer, "frozen {}", 0)?;
                }
                _ => {
                    // TODO: Add support for reading other attributes.
                    return Err(Error::AttributeError);
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

                with_process_cgroup_locked(pid, |target_process, cgroup_membership| {
                    // TODO: According to the "no internal processes" rule of cgroupv2
                    // (Ref: https://man7.org/linux/man-pages/man7/cgroups.7.html),
                    // if the cgroup node has enabled some controllers like "memory", "io",
                    // it is forbidden to bind a process to an internal cgroup node.
                    cgroup_membership.move_process_to_node(target_process, self)
                })?;

                Ok(len)
            }
            _ => {
                // TODO: Add support for writing other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.with_inner(|_| {
            let new_child = CgroupNode::new(name.to_string().into(), self.depth + 1);
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
fn with_process_cgroup_locked<F>(pid: Pid, op: F) -> Result<()>
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
