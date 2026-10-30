// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Error, SysAttrSet, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use inherit_methods_macro::inherit_methods;
use ostd::prelude::ktest;

use super::{config_root, register_subsystem};

#[derive(Debug)]
struct TestSubsystem {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl TestSubsystem {
    fn new(name: &'static str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(name),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }
}

inherit_sys_branch_node!(TestSubsystem, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[ktest]
fn register_configfs_subsystem() {
    const NAME: &str = "configfs_ktest_subsystem";

    let root = config_root();
    assert!(root.child(NAME).is_none());

    register_subsystem(TestSubsystem::new(NAME)).unwrap();
    assert!(root.child(NAME).is_some());

    let duplicate = register_subsystem(TestSubsystem::new(NAME));
    assert!(matches!(duplicate, Err(Error::AlreadyExists)));
}
