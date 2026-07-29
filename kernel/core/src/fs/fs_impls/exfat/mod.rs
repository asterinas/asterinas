// SPDX-License-Identifier: MPL-2.0

mod bitmap;
mod constants;
mod dentry;
mod fat;
mod fs;
mod inode;
mod super_block;
mod upcase_table;
mod utils;

use crate::fs::exfat::fs::ExfatType;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&ExfatType).unwrap();
}
