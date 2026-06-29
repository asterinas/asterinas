// SPDX-License-Identifier: MPL-2.0

//! VFS adapters for filesystems provided by the `hadris` crates.

mod block_io;
mod fat;
mod iso9660;

pub(super) fn init() {
    fat::init();
    iso9660::init();
}
