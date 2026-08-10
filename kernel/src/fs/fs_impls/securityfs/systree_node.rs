// SPDX-License-Identifier: MPL-2.0

use aster_systree::{
    BranchNodeFields, SysBranchNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};

use crate::{prelude::*, security::lsm};

pub(super) struct SecurityRootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl SecurityRootNode {
    pub(super) fn new() -> Arc<Self> {
        let root = Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(SysStr::from(""), Default::default(), weak_self.clone()),
        });

        for node in lsm::securityfs_nodes() {
            root.fields
                .add_child(node)
                .expect("LSM securityfs node names must be unique");
        }

        root
    }
}

impl Debug for SecurityRootNode {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SecurityRootNode")
            .finish_non_exhaustive()
    }
}

inherit_sys_branch_node!(SecurityRootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {}

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
