// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Result, SysAttrSet, SysBranchNode, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node,
};
use inherit_methods_macro::inherit_methods;
use spin::Once;

/// The `SysTree` node that represents the root node of the `ConfigFs`.
#[derive(Debug)]
pub struct ConfigRootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ConfigRootNode {
    /// Returns the `ConfigRootNode` singleton.
    pub(super) fn singleton() -> &'static Arc<ConfigRootNode> {
        static SINGLETON: Once<Arc<ConfigRootNode>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
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
