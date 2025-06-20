// SPDX-License-Identifier: MPL-2.0

//! A global `memfd` file manager.

// TODO: Unify `MemfdManager` and `SharedMemManager` into a single struct.
// They can share a `RamFS` backend.

use alloc::format;

use spin::Once;

use crate::{
    fs::{
        inode_handle::InodeHandle,
        ramfs::RamFS,
        utils::{AccessMode, InodeMode, InodeType, StatusFlags},
    },
    prelude::*,
};

/// Maximum file name length for `memfd_create`, excluding the final `\0` byte.
///
/// See <https://man7.org/linux/man-pages/man2/memfd_create.2.html>
pub const MAX_MEMFD_NAME_LEN: usize = 249;

pub struct MemfdManager {
    /// Ramfs as the underlying storage for `memfd` files.
    backend: Arc<RamFS>,
}

pub static MEMFD_MANAGER: Once<MemfdManager> = Once::new();

pub fn init() {
    MEMFD_MANAGER.call_once(MemfdManager::new);
}

impl MemfdManager {
    #[expect(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            backend: RamFS::new(),
        }
    }

    pub fn create(&self, name: &str) -> Result<InodeHandle> {
        if name.len() > MAX_MEMFD_NAME_LEN {
            return_errno_with_message!(Errno::EINVAL, "MemfdManager: `name` is too long.");
        }

        let memfd_name = format!("memfd:{}", name);

        let inode = self
            .backend
            .create_detached(InodeType::File, InodeMode::from_bits_truncate(0o600))?;

        InodeHandle::new_unchecked_access_with_inode(
            inode,
            &memfd_name,
            AccessMode::O_RDWR,
            StatusFlags::empty(),
        )
    }
}
