// SPDX-License-Identifier: MPL-2.0

use super::{RawInode, block_mapping::BlockMapping};
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*};

/// Final deleted-inode work retained by EXT after the last inode handle drops.
/// The inode number remains allocated until both mapping cleanup and inode
/// writeback succeed. The mapping holds only a weak filesystem reference.
#[derive(Debug)]
pub(in crate::fs::fs_impls::ext4) struct PendingInodeWork {
    pub(super) ino: Ext4Ino,
    pub(super) type_: InodeType,
    pub(super) raw: RawInode,
    pub(super) mapping: Arc<BlockMapping>,
    pub(super) xattr: Option<crate::fs::ext4::xattr::Xattr>,
}

impl PendingInodeWork {
    pub(in crate::fs::fs_impls::ext4) fn retry(&mut self, fs: &Ext4) -> Result<()> {
        if let Some(xattr) = &self.xattr {
            xattr.delete_xattr_block()?;
        }
        self.raw.file_acl = 0;
        self.mapping.truncate_to_byte_len(0)?;
        self.mapping.sync_metadata()?;
        let raw = self.mapping.raw_block_ptrs();
        self.raw.block = raw.block_ptrs;
        self.raw.sector_count = raw.sector_count;
        fs.write_back_inode_desc(self.ino, &self.raw)?;
        self.mapping.clear_dirty();
        fs.free_inode(self.ino, self.type_)
    }
}
