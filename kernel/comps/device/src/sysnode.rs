// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    string::ToString,
    sync::{Arc, Weak},
};

use aster_systree::{
    inherit_sys_branch_node, inherit_sys_symlink_node, AttrLessBranchNodeFields, Result,
    SymlinkNodeFields, SysBranchNode, SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;

use super::Device;

/// The `dev` node in sysfs.
///
/// This structure represents the `dev` directory in the sysfs filesystem.
/// It contains subdirectories for different device types (e.g., block, char)
/// and symlinks to individual devices.
#[derive(Debug)]
pub struct DevNode {
    fields: AttrLessBranchNodeFields<DevSymlinks, Self>,
}

inherit_sys_branch_node!(DevNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[inherit_methods(from = "self.fields")]
impl DevNode {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<DevSymlinks>>;
    pub fn add_child(&self, new_child: Arc<DevSymlinks>) -> Result<()>;
    pub fn remove_child(&self, child_name: &str) -> Result<Arc<DevSymlinks>>;
}

impl DevNode {
    /// Creates a new `DevNode`.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: AttrLessBranchNodeFields::new(SysStr::from("dev"), weak_self.clone()),
        })
    }
}

/// A collection of device symlinks in sysfs.
///
/// This structure represents a directory in sysfs that contains symlinks to devices
/// of a specific type (e.g., block devices or character devices).
#[derive(Debug)]
pub struct DevSymlinks {
    fields: AttrLessBranchNodeFields<DevSymlink, Self>,
}

inherit_sys_branch_node!(DevSymlinks, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[inherit_methods(from = "self.fields")]
impl DevSymlinks {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<DevSymlink>>;
    pub fn add_child(&self, new_child: Arc<DevSymlink>) -> Result<()>;
    pub fn remove_child(&self, child_name: &str) -> Result<Arc<DevSymlink>>;
}

impl DevSymlinks {
    /// Creates a new `DevSymlinks` node with the specified name.
    ///
    /// This is used internally to create directories for different device types
    /// (e.g., "block", "char") under `/dev`.
    pub(crate) fn new(name: &str) -> Arc<Self> {
        let name = SysStr::from(name.to_string());
        Arc::new_cyclic(|weak_self| Self {
            fields: AttrLessBranchNodeFields::new(name, weak_self.clone()),
        })
    }
}

/// A symlink to a device in sysfs.
///
/// This structure represents a symbolic link in sysfs that points to a device's
/// sysfs node. The symlink is named using the device's major and minor numbers
/// (e.g., "1:3" for a device with major number 1 and minor number 3).
#[derive(Debug)]
pub struct DevSymlink {
    device: Weak<dyn Device>,
    field: SymlinkNodeFields<Self>,
}

inherit_sys_symlink_node!(DevSymlink, field);

impl DevSymlink {
    /// Creates a new `DevSymlink` pointing to the specified device.
    pub fn new(name: &str, device: &Arc<dyn Device>) -> Arc<Self> {
        let name = SysStr::from(name.to_string());
        let target_path = format!("../..{}", device.sysnode().path());
        Arc::new_cyclic(|weak_self| Self {
            device: Arc::downgrade(device),
            field: SymlinkNodeFields::new(name, target_path, weak_self.clone()),
        })
    }

    /// Retrieves the device associated with this symlink.
    pub fn device(&self) -> Option<Arc<dyn Device>> {
        let device = self.device.upgrade();
        if device.is_some() {
            return device;
        }

        if let Some(parent) = self.parent() {
            // Remove the invalid symlink from its parent.
            let _ = parent.remove_child(self.name());
        };

        None
    }
}

/// The `devices` node in sysfs.
///
/// This structure represents the `/devices` directory in the sysfs filesystem.
/// It contains entries for all devices registered with the system.
#[derive(Debug)]
pub struct DevicesNode {
    fields: AttrLessBranchNodeFields<dyn SysBranchNode, Self>,
}

inherit_sys_branch_node!(DevicesNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[inherit_methods(from = "self.fields")]
impl DevicesNode {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> Result<()>;
    pub fn remove_child(&self, child_name: &str) -> Result<Arc<dyn SysBranchNode>>;
}

impl DevicesNode {
    /// Creates a new `DevicesNode`.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: AttrLessBranchNodeFields::new(SysStr::from("devices"), weak_self.clone()),
        })
    }
}
