// SPDX-License-Identifier: MPL-2.0

use fs::OverlayFsType;

mod fs;

pub(super) fn init() {
    super::registry::register(&OverlayFsType).unwrap();
}
