// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{Error, Result, SysAttrSetBuilder, SysPerms, SysStr};
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
            builder.add(SysStr::from("memory.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);
            builder.add(SysStr::from("memory.max"), SysPerms::DEFAULT_RO_ATTR_PERMS);
            builder.add(
                SysStr::from("memory.events"),
                SysPerms::DEFAULT_RO_ATTR_PERMS,
            );
        }
    }
}

impl super::SubControl for MemoryController {
    fn read_attr_at(&self, _name: &str, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl super::SubControlStatic for MemoryController {
    fn new(_is_root: bool) -> Self {
        Self { _private: () }
    }

    fn type_() -> super::SubCtrlType {
        super::SubCtrlType::Memory
    }

    fn read_from(controller: &super::Controller) -> Arc<super::SubController<Self>> {
        controller.memory.read().get().clone()
    }
}
