// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(ext4)]

use super::{RawInode, block_mapping::BlockMapping};
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*};

/// Final deleted-inode work retained by EXT after the last inode handle drops.
/// The inode number remains allocated until both mapping cleanup and inode
/// writeback succeed. Inodes without data mappings still need these retries.
/// Any retained mapping holds only a weak filesystem reference.
#[derive(Debug)]
pub(in ext4) struct PendingInodeWork {
    pub(super) ino: Ext4Ino,
    pub(super) type_: InodeType,
    pub(super) raw: RawInode,
    pub(super) mapping: Option<Arc<BlockMapping>>,
    pub(super) xattr: Option<crate::fs::ext4::xattr::Xattr>,
}

impl PendingInodeWork {
    pub(in ext4) fn retry(&mut self, fs: &Ext4) -> Result<()> {
        if let Some(xattr) = &self.xattr {
            xattr.delete_xattr_block()?;
        }
        self.raw.file_acl = 0;
        if let Some(mapping) = &self.mapping {
            mapping.truncate_to_byte_len(0)?;
            mapping.sync_metadata()?;
            let raw = mapping.raw_block_ptrs();
            self.raw.block = raw.block_ptrs;
            self.raw.sector_count = raw.sector_count;
        } else {
            // Without a mapping, only xattrs could have contributed sectors.
            self.raw.sector_count = 0;
        }
        fs.write_back_inode_desc(self.ino, &self.raw)?;
        if let Some(mapping) = &self.mapping {
            mapping.clear_inode_dirty();
        }
        fs.free_inode(self.ino, self.type_)
    }
}
