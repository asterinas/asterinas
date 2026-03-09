// SPDX-License-Identifier: MPL-2.0

use fs::OverlayFsType;

mod fs;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&OverlayFsType).unwrap();
}
