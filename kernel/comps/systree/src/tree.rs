// SPDX-License-Identifier: MPL-2.0

//! Defines the main `SysTree` structure and its root node implementation.

use alloc::sync::{Arc, Weak};

use inherit_methods_macro::inherit_methods;

use super::{
    attr::SysAttrSet,
    node::{SysBranchNode, SysObj},
    Result, SysStr,
};
use crate::{inherit_sys_branch_node, BranchNodeFields, SysPerms};

#[derive(Debug)]
pub struct SysTree<Root: SysBranchNode> {
    root: Arc<Root>,
    // event_hub: SysEventHub,
}

impl SysTree<RootNode> {
    /// Creates a new `SysTree` instance with a default root node.
    pub(crate) fn new() -> Self {
        let name = ""; // Only the root has an empty name
        let attr_set = SysAttrSet::new_empty(); // The root has no attributes

        let root_node = Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(SysStr::from(name), attr_set, weak_self.clone());
            RootNode { fields }
        });

        Self { root: root_node }
    }
}

impl<Root: SysBranchNode> SysTree<Root> {
    /// Creates a new `SysTree` instance with the provided root node.
    pub fn new_with_root(root: Arc<Root>) -> Result<Self> {
        if !root.is_root() {
            return Err(crate::Error::InternalError(
                "The provided root is not a root node",
            ));
        }

        Ok(Self { root })
    }

    /// Returns a reference to the root node of the tree.
    pub fn root(&self) -> &Arc<Root> {
        &self.root
    }
}

/// The default root node in the `SysTree`.
///
/// A `RootNode` can work like a branching node, allowing to add additional nodes
/// as its children.
#[derive(Debug)]
pub struct RootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl RootNode {
    /// Adds a child node to this `RootNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(RootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {
        // This method should be a no-op for `RootNode`.
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
