// SPDX-License-Identifier: MPL-2.0

//! Ext4 feature flags and the masks of features this implementation supports.
//!
//! Features are split into three classes per the ext4 on-disk format: compatible
//! (`s_feature_compat`), incompatible (`s_feature_incompat`), and read-only
//! compatible (`s_feature_ro_compat`). Mount handling uses the `*_SUPP` masks to
//! decide whether to reject a volume: an unsupported `incompat` feature is
//! refused outright, and an unsupported `ro_compat` feature is refused with
//! `EROFS` (this read-only port cannot interpret such a feature's on-disk
//! semantics, so it refuses the volume rather than misread it).
//!
//! Naming follows ext2's de-prefixed style; each flag's doc comment cites the
//! Linux `EXT4_FEATURE_*` constant for cross-reference.

use super::prelude::*;

bitflags! {
    /// Compatible features. An unknown bit here does not affect mounting.
    pub(super) struct FeatureCompatSet: u32 {
        /// `COMPAT_DIR_PREALLOC`.
        const DIR_PREALLOC = 1 << 0;
        /// `COMPAT_IMAGIC_INODES`.
        const IMAGIC_INODES = 1 << 1;
        /// `COMPAT_HAS_JOURNAL`: the volume has a journal (inode 8).
        const HAS_JOURNAL = 1 << 2;
        /// `COMPAT_EXT_ATTR`: inodes have extended attributes.
        const EXT_ATTR = 1 << 3;
        /// `COMPAT_RESIZE_INODE`.
        const RESIZE_INODE = 1 << 4;
        /// `COMPAT_DIR_INDEX`: directories use an htree hash index.
        const DIR_INDEX = 1 << 5;
    }
}

bitflags! {
    /// Incompatible features. An unknown bit not in `INCOMPAT_SUPP` makes the
    /// volume unmountable.
    pub(super) struct FeatureIncompatSet: u32 {
        /// `INCOMPAT_COMPRESSION`.
        const COMPRESSION = 1 << 0;
        /// `INCOMPAT_FILETYPE`: directory entries record the file type.
        const FILETYPE = 1 << 1;
        /// `INCOMPAT_RECOVER`: the journal needs replay before writing.
        const RECOVER = 1 << 2;
        /// `INCOMPAT_JOURNAL_DEV`: journal is on an external device.
        const JOURNAL_DEV = 1 << 3;
        /// `INCOMPAT_META_BG`.
        const META_BG = 1 << 4;
        /// `INCOMPAT_EXTENTS`: inodes use extent block mapping (required).
        const EXTENTS = 1 << 6;
        /// `INCOMPAT_64BIT`: 64-bit block numbers and 64-byte group descriptors.
        const IS_64BIT = 1 << 7;
        /// `INCOMPAT_MMP`: multiple-mount protection.
        const MMP = 1 << 8;
        /// `INCOMPAT_FLEX_BG`: flexible block groups.
        const FLEX_BG = 1 << 9;
        /// `INCOMPAT_EA_INODE`.
        const EA_INODE = 1 << 10;
        /// `INCOMPAT_DIRDATA`.
        const DIRDATA = 1 << 12;
        /// `INCOMPAT_CSUM_SEED`: checksum seed stored in the superblock.
        const CSUM_SEED = 1 << 13;
        /// `INCOMPAT_LARGEDIR`.
        const LARGEDIR = 1 << 14;
        /// `INCOMPAT_INLINE_DATA`: small files stored inline in the inode.
        const INLINE_DATA = 1 << 15;
        /// `INCOMPAT_ENCRYPT`.
        const ENCRYPT = 1 << 16;
        /// `INCOMPAT_CASEFOLD`.
        const CASEFOLD = 1 << 17;
    }
}

bitflags! {
    /// Read-only compatible features. A bit not in [`RO_COMPAT_SUPP`] refuses
    /// the mount with `EROFS` — see the constant's doc.
    pub(super) struct FeatureRoCompatSet: u32 {
        /// `RO_COMPAT_SPARSE_SUPER`: sparse superblock and GDT backups.
        const SPARSE_SUPER = 1 << 0;
        /// `RO_COMPAT_LARGE_FILE`: 64-bit file sizes.
        const LARGE_FILE = 1 << 1;
        /// `RO_COMPAT_HUGE_FILE`: `i_blocks` counted in filesystem blocks.
        const HUGE_FILE = 1 << 3;
        /// `RO_COMPAT_GDT_CSUM`: legacy block-group descriptor checksums.
        const GDT_CSUM = 1 << 4;
        /// `RO_COMPAT_DIR_NLINK`: directory link counts may exceed 65000.
        const DIR_NLINK = 1 << 5;
        /// `RO_COMPAT_EXTRA_ISIZE`: inodes record their used extra size.
        const EXTRA_ISIZE = 1 << 6;
        /// `RO_COMPAT_QUOTA`.
        const QUOTA = 1 << 8;
        /// `RO_COMPAT_BIGALLOC`.
        const BIGALLOC = 1 << 9;
        /// `RO_COMPAT_METADATA_CSUM`: crc32c checksums on metadata.
        const METADATA_CSUM = 1 << 10;
        /// `RO_COMPAT_PROJECT`.
        const PROJECT = 1 << 13;
        /// `RO_COMPAT_VERITY`.
        const VERITY = 1 << 15;
    }
}

/// Incompatible features this implementation supports.
///
/// The reader understands typed directory entries (`FILETYPE`) and extent block
/// mapping (`EXTENTS`). `RECOVER` is deliberately **not** in this mask: this
/// read-only mount carries no journal-replay machinery, so a volume left with
/// the journal needing replay is a dirty volume and must be rejected (its
/// on-disk bytes may be a pre-crash inconsistent state the funnel would serve
/// verbatim), not mounted and silently read without replay. Phase 6 adds
/// `FLEX_BG`: flex_bg only relocates each group's bitmaps and inode table, and
/// those block numbers already come from the descriptor getters
/// (`block_bitmap_bid` etc.), never from geometry, so the read/write paths need
/// no change beyond admitting the bit. Phase 6 also adds `IS_64BIT`: 64-byte
/// group descriptors and the `s_*_count_hi` superblock halves are decoded wide
/// (`BlockGroup::read_desc` strides by `s_desc_size` and splices the block-number
/// high halves; `SuperBlock::try_from` splices the count high halves). This bit
/// joins the mask **atomically with that decoder** — admitting it before the wide
/// decode existed would misread every group descriptor as a truncated address.
/// Other incompatible features (csum_seed, ...) are added in later phases and
/// must stay out of this mask until then, so an image needing them is rejected
/// rather than silently misread. (`HAS_JOURNAL` is a *compat* feature — an
/// unknown/ignored compat bit on this no-journal read-only mount — so it does
/// not belong in this incompat mask.)
pub(super) const INCOMPAT_SUPP: FeatureIncompatSet = FeatureIncompatSet::FILETYPE
    .union(FeatureIncompatSet::EXTENTS)
    .union(FeatureIncompatSet::FLEX_BG)
    .union(FeatureIncompatSet::IS_64BIT);

/// Read-only compatible features this implementation handles.
///
/// Most need no extra code to read correctly. `METADATA_CSUM` is the
/// exception: its crc32c is verified on read for the superblock, the group
/// descriptors, and inodes. Bitmap, directory-block, and extent-node checksums
/// are not verified on read (a known limitation); such a volume still mounts
/// read-only. Known limitation: for an inode carrying `EXT4_HUGE_FILE_FL`,
/// `i_blocks` is in filesystem-block units; this reader treats it as 512-byte
/// sectors, so `st_blocks` may be under-reported for such inodes (normally
/// files larger than 2 TiB, though a corrupt image could set the flag on any
/// inode). The file data read back is unaffected. A volume carrying any
/// `ro_compat` bit still outside this mask carries semantics this reader cannot
/// interpret, so `SuperBlock::try_from` refuses the mount with `EROFS` (Linux
/// `ext4_setup_super` parity).
pub(super) const RO_COMPAT_SUPP: FeatureRoCompatSet = FeatureRoCompatSet::SPARSE_SUPER
    .union(FeatureRoCompatSet::LARGE_FILE)
    .union(FeatureRoCompatSet::HUGE_FILE)
    .union(FeatureRoCompatSet::DIR_NLINK)
    .union(FeatureRoCompatSet::EXTRA_ISIZE)
    .union(FeatureRoCompatSet::METADATA_CSUM);
