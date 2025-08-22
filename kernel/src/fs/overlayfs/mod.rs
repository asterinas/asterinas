// SPDX-License-Identifier: MPL-2.0

mod fs;

use alloc::sync::Arc;

use crate::fs::overlayfs::fs::OverlayFsType;

pub(super) fn init() {
    let overlay_type = Arc::new(OverlayFsType);
    super::registry::register(overlay_type).unwrap();
}
