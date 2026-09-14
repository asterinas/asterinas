// SPDX-License-Identifier: MPL-2.0

//! Registration interfaces for file systems and kernel models backed by SysTree.

mod kernel;

use alloc::sync::Arc;

use aster_systree::{SysBranchNode, SysNode};

use super::{utils::systree_fs::SingletonSysTreeFsType, vfs::registry};
pub use crate::error::Error;

/// The result type used by SysTree registration operations.
pub type Result<T> = core::result::Result<T, Error>;

/// A registration descriptor for a singleton file system backed by a fixed SysTree root.
///
/// The file system uses the kernel's default SysTree-to-VFS adapter. Create
/// requests are forwarded to the model, while removal and rename requests are
/// rejected. File systems with per-mount roots or custom mutation semantics
/// require a dedicated interface.
pub struct SingletonSysTreeFs(SingletonSysTreeFsType);

impl SingletonSysTreeFs {
    /// Creates a singleton SysTree file-system descriptor.
    pub const fn new(
        name: &'static str,
        magic: u64,
        block_size: usize,
        name_max: usize,
        root: fn() -> Arc<dyn SysBranchNode>,
    ) -> Self {
        Self(SingletonSysTreeFsType::new(
            name, magic, block_size, name_max, root,
        ))
    }

    /// Registers the file-system type with the kernel VFS.
    ///
    /// This method may only be called once for a given file-system name.
    ///
    /// # Panics
    ///
    /// Panics if the VFS registry is not initialized.
    pub fn register(&'static self) -> Result<()> {
        registry::register(&self.0)
    }
}

pub(super) fn init() {
    kernel::init();
}

/// Registers a model node under the primary SysTree's `kernel` namespace.
///
/// Views of the primary tree expose registered nodes according to their own
/// mount and rendering rules.
///
/// # Panics
///
/// Panics if the primary SysTree's `kernel` namespace is not initialized.
pub fn register_kernel_node(node: Arc<dyn SysNode>) -> Result<()> {
    kernel::register(node)
}
