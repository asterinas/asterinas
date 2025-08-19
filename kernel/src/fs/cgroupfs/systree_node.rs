// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    string::ToString,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode,
    SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use crate::{
    prelude::*,
    process::{process_table, Pid, Process},
};

/// The root of a cgroup hierarchy, serving as the entry point to
/// the entire cgroup control system.
///
/// The cgroup system provides v2 unified hierarchy, and is also used as a root
/// node in the cgroup systree.
#[derive(Debug)]
pub struct CgroupSystem {
    fields: BranchNodeFields<CgroupNode, Self>,
}

/// A control group node in the cgroup systree.
///
/// Each node can bind a group of processes together for purpose of resource
/// management. Except for the root node, all nodes in the cgroup tree are of
/// this type.
pub struct CgroupNode {
    fields: BranchNodeFields<CgroupNode, Self>,
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
    pub(super) fn new(name: SysStr, depth: usize) -> Arc<Self> {
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
                processes: Mutex::new(BTreeMap::new()),
                depth,
                populated_count: AtomicUsize::new(0),
            }
        })
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
    fn read_procs(&self, writer: &mut VmWriter) -> Result<usize> {
        let context = self
            .processes
            .lock()
            .keys()
            .map(|pid| pid.to_string())
            .collect::<Vec<String>>()
            .join("\n");

        writer
            .write_fallible(&mut VmReader::from((context + "\n").as_bytes()))
            .map_err(|_| Error::AttributeError)
    }

    /// Writes the PID of a process to this cgroup node.
    ///
    /// The corresponding process will be bound to this cgroup.
    /// The cgroup only allows binding one process at a time.
    fn write_procs(&self, reader: &mut VmReader) -> Result<usize> {
        // TODO: According to the "no internal processes" rule of cgroupv2
        // (Ref: https://man7.org/linux/man-pages/man7/cgroups.7.html),
        // if the cgroup node has enabled some controllers like "memory", "io",
        // it is forbidden to bind a process to an internal cgroup node.
        let (pid, pid_len) = read_pid_from_reader(reader)?;

        let process = if pid == 0 {
            current!()
        } else {
            process_table::get_process(pid).ok_or(Error::AttributeError)?
        };

        self.move_process(process);

        Ok(pid_len)
    }
}

inherit_sys_branch_node!(CgroupSystem, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {
        // This method should be a no-op for `RootNode`.
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let process_table = process_table::process_table_mut();
                let context = process_table
                    .iter()
                    .filter_map(|process| {
                        if process.cgroup().is_none() {
                            Some(process.pid().to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                writer
                    .write_fallible(&mut VmReader::from((context + "\n").as_bytes()))
                    .map_err(|_| Error::AttributeError)
            }
            _ => {
                // TODO: Add support for reading other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let (pid, pid_len) = read_pid_from_reader(reader)?;

                let process = if pid == 0 {
                    current!()
                } else {
                    process_table::get_process(pid).ok_or(Error::AttributeError)?
                };

                let rcu_old_cgroup = process.cgroup();
                let old_cgroup = rcu_old_cgroup.get();
                if let Some(old_cgroup) = old_cgroup {
                    old_cgroup.remove_process(&process);
                }

                Ok(pid_len)
            }
            _ => {
                // TODO: Add support for reading other attributes.
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
    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "cgroup.procs" => self.read_procs(writer),
            "cgroup.events" => {
                let res = if self.populated_count.load(Ordering::Acquire) > 0 {
                    1
                } else {
                    0
                };
                // Currently we have not enabled the "frozen" attribute
                // so the "frozen" field is always zero.
                let output = format!("populated {}\nfrozen {}\n", res, 0);
                writer
                    .write_fallible(&mut VmReader::from(output.as_bytes()))
                    .map_err(|_| Error::AttributeError)
            }
            _ => {
                // TODO: Add support for reading other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => self.write_procs(reader),
            _ => {
                // TODO: Add support for reading other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let new_child = CgroupNode::new(name.to_string().into(), self.depth + 1);
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});

/// Reads a PID from the given reader.
///
/// Returns a tuple containing the PID and the number of bytes read.
fn read_pid_from_reader(reader: &mut VmReader) -> Result<(Pid, usize)> {
    let mut pid_buffer = alloc::vec![0; reader.remain()];
    let pid_len = reader
        .read_fallible(&mut VmWriter::from(pid_buffer.as_mut_slice()))
        .map_err(|_| Error::AttributeError)?;

    let pid = alloc::str::from_utf8(&pid_buffer[..pid_len])
        .map_err(|_| Error::AttributeError)
        .and_then(|string| {
            let strip_string = string.trim();
            strip_string
                .parse::<u32>()
                .map_err(|_| Error::AttributeError)
        })?;

    Ok((pid, pid_len))
}
