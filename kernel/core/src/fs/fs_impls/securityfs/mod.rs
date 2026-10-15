// SPDX-License-Identifier: MPL-2.0

//! securityfs filesystem implementation.
//!
//! This module connects the public securityfs registration seam to an
//! immutable [`aster_systree`] hierarchy and the VFS inode adapter.

mod fs;
mod inode;
mod node;
#[cfg(ktest)]
mod test;

use aster_systree::EmptyNode;
use fs::SecurityFsType;
use node::{SecurityDirectory, SecurityRootNode};

use crate::{fs::vfs::registry, prelude::*, security::securityfs::SecurityFsFile};

pub(super) fn init() {
    SecurityRootNode::singleton();
    super::sysfs::register_kernel_sysnode(EmptyNode::new("security".into())).unwrap();
    registry::register(&SecurityFsType).unwrap();
}

pub(crate) fn register_directory(
    name: &'static str,
    files: &'static [SecurityFsFile],
) -> Result<()> {
    SecurityRootNode::singleton().add_child(SecurityDirectory::new(name, files)?)?;
    Ok(())
}

#[cfg(ktest)]
pub(crate) fn init_for_ktest() {
    use spin::Once;

    static INIT: Once<()> = Once::new();
    INIT.call_once(|| {
        crate::time::clocks::init_for_ktest();
        aster_systree::init_for_ktest();
        crate::fs::vfs::init();
        if registry::look_up("sysfs").is_none() {
            super::sysfs::init();
        }
        init();
    });
}
