// SPDX-License-Identifier: MPL-2.0

//! Temporary file system (tmpfs) based on RamFs.

pub(super) use fs::TmpFs;
use fs::TmpFsType;

mod fs;

#[expect(dead_code)]
const TMPFS_MAGIC: u64 = 0x0102_1994;

pub(super) fn init() {
    super::registry::register(&TmpFsType).unwrap();
}
