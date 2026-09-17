// SPDX-License-Identifier: MPL-2.0

//! Classic EXT direct and indirect block-pointer mapping.

mod block_ptr_tree;
mod indirect_block_manager;

pub(super) use self::block_ptr_tree::ResolvedBlockRange;
pub(in crate::fs::fs_impls::ext4::inode) use self::block_ptr_tree::{BlockPtrTree, RawBlockPtrs};
