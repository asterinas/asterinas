// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
use ostd::mm::{VmReader, VmWriter};

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
    fn read_attr_at(&self, _name: &str, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
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
