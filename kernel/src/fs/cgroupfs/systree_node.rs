// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::btree_map::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::fmt::Debug;

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysAttrSetBuilder, SysBranchNode,
    SysBranchNodeFields, SysMode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    sync::Mutex,
};

use crate::{
    current,
    process::{process_table, Pid, Process},
};

/// A node in the cgroup systree, which represents the unified cgroup node.
///
/// This kind of node is used in the v2 unified hierarchy as the root of the cgroup tree.
#[derive(Debug)]
pub struct CgroupUnifiedNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

/// A node in the cgroup systree, which represents a normal cgroup node.
///
/// Except for the root node, all nodes in the cgroup tree are of this type.
pub struct CgroupNormalNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    processes: Mutex<BTreeMap<Pid, Arc<Process>>>,
    weak_self: Weak<Self>,
}

impl Debug for CgroupNormalNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupNormalNode")
            .field("fields", &self.fields)
            .finish()
    }
}

#[inherit_methods(from = "self.fields")]
impl CgroupUnifiedNode {
    /// Adds a child node to this `CgroupUnifiedNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(name) {
            return Err(Error::PermissionDenied);
        }

        new_child.set_parent_path(SysStr::from(""));
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }

    pub fn remove_child(&self, child_name: &str) -> Option<Arc<dyn SysObj>>;
}

#[inherit_methods(from = "self.fields")]
impl CgroupNormalNode {
    /// Adds a child node to this `CgroupNormalNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;

    pub fn remove_child(&self, child_name: &str) -> Option<Arc<dyn SysObj>>;
}

impl CgroupUnifiedNode {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysMode::DEFAULT_RO_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cgroup.procs"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(
            SysStr::from("cgroup.threads"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cpu.pressure"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(SysStr::from("cpu.stat"), SysMode::DEFAULT_RO_ATTR_MODE);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupUnifiedNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

impl CgroupNormalNode {
    pub(super) fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysMode::DEFAULT_RO_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cgroup.procs"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(
            SysStr::from("cgroup.threads"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cpu.pressure"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(SysStr::from("cpu.stat"), SysMode::DEFAULT_RO_ATTR_MODE);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupNormalNode {
            fields,
            processes: Mutex::new(BTreeMap::new()),
            weak_self: weak_self.clone(),
        })
    }
}

// Process-related operations.
impl CgroupNormalNode {
    /// Binds a process to this cgroup node.
    ///
    /// A process can only be bound to one cgroup at a time.
    /// If the process is already bound to another cgroup, it will
    /// be removed from that cgroup.
    pub fn bind_process(&self, process: Arc<Process>) {
        let old_cgroup = process.bind_cgroup(Some(self.weak_self.clone()));
        if let Some(old_cgroup) = old_cgroup {
            old_cgroup.remove_process(process.pid());
        }

        self.processes.lock().insert(process.pid(), process);
    }

    /// Removes a process from this cgroup node.
    pub fn remove_process(&self, pid: Pid) {
        self.processes.lock().remove(&pid);
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
        let (pid, pid_len) = read_pid_from_reader(reader)?;

        let process = if pid == 0 {
            current!()
        } else {
            process_table::get_process(pid).ok_or(Error::AttributeError)?
        };

        self.bind_process(process);

        Ok(pid_len)
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupUnifiedNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        true
    }

    fn path(&self) -> SysStr {
        SysStr::from("/")
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupNormalNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        false
    }

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for CgroupUnifiedNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
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

                let old_cgroup = process.bind_cgroup(None);
                if let Some(old_cgroup) = old_cgroup {
                    old_cgroup.remove_process(process.pid());
                }

                Ok(pid_len)
            }
            _ => {
                // TODO: Add support for reading other attributes.
                Err(Error::AttributeError)
            }
        }
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

impl SysNode for CgroupNormalNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "cgroup.procs" => self.read_procs(writer),
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

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupUnifiedNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupNormalNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}

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
