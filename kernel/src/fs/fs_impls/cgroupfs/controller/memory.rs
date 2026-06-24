// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
use aster_util::printer::VmPrinter;
use ostd::mm::{VmReader, VmWriter};

/// A sub-controller responsible for memory resource management in the cgroup subsystem.
///
/// Note that even if the controller is inactive, it still provides some interfaces
/// like "memory.pressure" for usage.
pub struct MemoryController {
    _private: (),
}

impl MemoryController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        // These attributes only exist on the non-root cgroup nodes.
        // However, it seems that the `memory.stat` attribute is also present on the root node in practice.
        // Currently the implementation follows the documentation strictly.
        //
        // Reference: <https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#memory-interface-files>
        if !is_root {
            builder.add(
                SysStr::from("memory.events"),
                SysPerms::DEFAULT_RO_ATTR_PERMS,
            );
            builder.add(SysStr::from("memory.max"), SysPerms::DEFAULT_RO_ATTR_PERMS);
            builder.add(SysStr::from("memory.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        }
    }
}

impl super::SubControl for MemoryController {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "memory.events" => {
                writeln!(printer, "low 0")?;
                writeln!(printer, "high 0")?;
                writeln!(printer, "max 0")?;
                writeln!(printer, "oom 0")?;
                writeln!(printer, "oom_kill 0")?;
                writeln!(printer, "oom_group_kill 0")?;
            }
            "memory.max" => {
                writeln!(printer, "max")?;
            }
            "memory.stat" => {
                writeln!(printer, "anon 0")?;
                writeln!(printer, "file 0")?;
                writeln!(printer, "kernel 0")?;
                writeln!(printer, "kernel_stack 0")?;
                writeln!(printer, "pagetables 0")?;
                writeln!(printer, "percpu 0")?;
                writeln!(printer, "sock 0")?;
                writeln!(printer, "shmem 0")?;
                writeln!(printer, "file_mapped 0")?;
                writeln!(printer, "file_dirty 0")?;
                writeln!(printer, "file_writeback 0")?;
                writeln!(printer, "swapcached 0")?;
                writeln!(printer, "anon_thp 0")?;
                writeln!(printer, "file_thp 0")?;
                writeln!(printer, "shmem_thp 0")?;
                writeln!(printer, "inactive_anon 0")?;
                writeln!(printer, "active_anon 0")?;
                writeln!(printer, "inactive_file 0")?;
                writeln!(printer, "active_file 0")?;
                writeln!(printer, "unevictable 0")?;
                writeln!(printer, "slab_reclaimable 0")?;
                writeln!(printer, "slab_unreclaimable 0")?;
                writeln!(printer, "slab 0")?;
                writeln!(printer, "workingset_refault_anon 0")?;
                writeln!(printer, "workingset_refault_file 0")?;
                writeln!(printer, "workingset_activate_anon 0")?;
                writeln!(printer, "workingset_activate_file 0")?;
                writeln!(printer, "workingset_restore_anon 0")?;
                writeln!(printer, "workingset_restore_file 0")?;
                writeln!(printer, "workingset_nodereclaim 0")?;
                writeln!(printer, "pgfault 0")?;
                writeln!(printer, "pgmajfault 0")?;
                writeln!(printer, "pgrefill 0")?;
                writeln!(printer, "pgscan 0")?;
                writeln!(printer, "pgsteal 0")?;
                writeln!(printer, "pgactivate 0")?;
                writeln!(printer, "pgdeactivate 0")?;
                writeln!(printer, "pglazyfree 0")?;
                writeln!(printer, "pglazyfreed 0")?;
                writeln!(printer, "thp_fault_alloc 0")?;
                writeln!(printer, "thp_collapse_alloc 0")?;
            }
            _ => return Err(Error::AttributeError),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }
}

impl super::SubControlStatic for MemoryController {
    fn new(_is_root: bool, _is_active: bool) -> Self {
        Self { _private: () }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Memory
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.memory.read().get().clone()
    }
}
