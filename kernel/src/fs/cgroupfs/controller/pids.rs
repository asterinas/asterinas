// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
use ostd::mm::{VmReader, VmWriter};

/// A sub-controller responsible for PID resource management in the cgroup subsystem.
///
/// This controller will only provide interfaces in non-root cgroup nodes.
pub struct PidsController {
    _private: (),
}

impl PidsController {
    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        if !is_root {
            builder.add(SysStr::from("pids.max"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        }
    }
}

impl super::SubControl for PidsController {
    fn read_attr_at(&self, _name: &str, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl super::SubControlStatic for PidsController {
    fn new(_is_root: bool) -> Self {
        Self { _private: () }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Pids
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.pids.read().get().clone()
    }
}
