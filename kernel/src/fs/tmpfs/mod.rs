// SPDX-License-Identifier: MPL-2.0

//! Temporary file system (tmpfs) based on RamFs.

use alloc::sync::Arc;

mod fs;

#[expect(dead_code)]
const TMPFS_MAGIC: u64 = 0x0102_1994;

pub(super) fn init() {
    let ramfs_type = Arc::new(fs::TmpFsType);
    super::registry::register(ramfs_type).unwrap();
}
