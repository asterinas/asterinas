// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Result, SysAttrSet, SysBranchNode, SysObj, SysPerms,
    SysStr,
};
use inherit_methods_macro::inherit_methods;

/// The `SysTree` node that represents the root node of the `ConfigFs`.
#[derive(Debug)]
pub struct ConfigRootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ConfigRootNode {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("config");

        let attrs = SysAttrSet::new_empty();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            ConfigRootNode { fields }
        })
    }

    /// Adds a child node.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(ConfigRootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {
        // This method should be a no-op for `RootNode`.
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
