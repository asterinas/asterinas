// SPDX-License-Identifier: MPL-2.0

//! Implementation of the `/sys/devices` sysfs directory.

use alloc::sync::Arc;

use aster_systree::{AttrLessBranchNodeFields, SysObj, SysPerms, SysStr, inherit_sys_branch_node};
use spin::Once;

pub(super) fn init() {
    DEVICES_SYS_NODE_ROOT.call_once(|| {
        let singleton = DevicesSysNodeRoot::new();
        super::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();

        singleton
    });
}

static DEVICES_SYS_NODE_ROOT: Once<Arc<DevicesSysNodeRoot>> = Once::new();

/// A systree node representing the `/sys/devices` directory.
///
/// This node serves as the top-level directory for device-related sysfs entries.
/// It corresponds to the `/devices` directory in the sysfs filesystem.
#[derive(Debug)]
struct DevicesSysNodeRoot {
    fields: AttrLessBranchNodeFields<dyn SysObj, Self>,
}

impl DevicesSysNodeRoot {
    /// Creates a new `DevicesSysNodeRoot` instance.
    fn new() -> Arc<Self> {
        let name = SysStr::from("devices");
        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            DevicesSysNodeRoot { fields }
        })
    }
}

inherit_sys_branch_node!(DevicesSysNodeRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
