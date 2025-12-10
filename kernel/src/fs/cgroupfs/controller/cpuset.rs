// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
use aster_util::printer::VmPrinter;
use ostd::{
    cpu::num_cpus,
    mm::{VmReader, VmWriter},
};

/// A sub-controller responsible for CPU resource management in the cgroup subsystem.
pub struct CpuSetController {
    _private: (),
}

impl CpuSetController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        if !is_root {
            builder.add(SysStr::from("cpuset.cpus"), SysPerms::DEFAULT_RW_ATTR_PERMS);
            builder.add(SysStr::from("cpuset.mems"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        }

        builder.add(
            SysStr::from("cpuset.cpus.effective"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cpuset.mems.effective"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
    }
}

impl super::SubControl for CpuSetController {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "cpuset.cpus.effective" => {
                let num_cpus = num_cpus();
                if num_cpus == 1 {
                    writeln!(printer, "0")?;
                } else {
                    writeln!(printer, "0-{}", num_cpus - 1)?;
                }
            }
            // Currently we only support a single memory node.
            "cpuset.mems.effective" => writeln!(printer, "0")?,
            _ => return Err(Error::AttributeError),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl super::SubControlStatic for CpuSetController {
    fn new(_is_root: bool) -> Self {
        Self { _private: () }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::CpuSet
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.cpuset.read().get().clone()
    }
}
