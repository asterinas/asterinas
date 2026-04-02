// SPDX-License-Identifier: MPL-2.0

mod fs;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs::VirtioFsType).unwrap();
}
