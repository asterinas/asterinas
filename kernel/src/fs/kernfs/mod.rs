// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

mod element;
mod inode;

use alloc::sync::Arc;

pub use element::{DataProvider, KernfsElem};
pub use inode::{KernfsNode, KernfsNodeFlag};

use super::utils::FileSystem;
use crate::prelude::*;

/// Block size.
const BLOCK_SIZE: usize = 1024;

/// A trait for the pseudo filesystem.
///
/// The pseudo filesystem is a virtual filesystem that is used to provide
/// a consistent interface for the kernel to interact with the underlying
/// hardware.
pub trait PseudoFileSystem: FileSystem {
    fn alloc_id(&self) -> u64;

    fn init(&self) -> Result<()>;

    fn fs(&self) -> Arc<dyn FileSystem>;
}
