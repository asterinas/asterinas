// SPDX-License-Identifier: MPL-2.0

//! Ext4 feature flags and the masks of features this implementation supports.
//!
//! Features are split into three classes per the ext4 on-disk format: compatible
//! (`s_feature_compat`), incompatible (`s_feature_incompat`), and read-only
//! compatible (`s_feature_ro_compat`). Mount handling uses the `*_SUPP` masks
//! to decide whether to reject a volume or downgrade it to read-only.
//!
//! Naming follows ext2's de-prefixed style.

use super::{fs::MountFlavor, prelude::*, super_block::SuperBlock};

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
    /// Read-only compatible features. An unknown bit not in `RO_COMPAT_SUPP`
    /// forces a read-only mount.
    pub(super) struct FeatureRoCompatSet: u32 {
        /// `RO_COMPAT_SPARSE_SUPER`: sparse superblock and GDT backups.
        const SPARSE_SUPER = 1 << 0;
        /// `RO_COMPAT_LARGE_FILE`: 64-bit file sizes.
        const LARGE_FILE = 1 << 1;
        /// `RO_COMPAT_BTREE_DIR`: historic btree-directory flag. Never given
        /// an on-disk meaning; accepted for parity with the classic ext2
        /// driver.
        const BTREE_DIR = 1 << 2;
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
/// Typed directory entries and extent block mapping are supported. A volume
/// requiring any other incompatible feature is rejected.
pub(super) const INCOMPAT_SUPP: FeatureIncompatSet =
    FeatureIncompatSet::FILETYPE.union(FeatureIncompatSet::EXTENTS);

/// Read-only compatible features this implementation handles.
///
/// These need no extra code to read correctly. This implementation has no
/// read-only mount mode to downgrade to, so a volume carrying any other
/// `ro_compat` bit is rejected: writing while ignoring such a feature would
/// corrupt it (e.g. `metadata_csum` volumes would silently accumulate stale
/// checksums).
pub(super) const RO_COMPAT_SUPP: FeatureRoCompatSet = FeatureRoCompatSet::SPARSE_SUPER
    .union(FeatureRoCompatSet::LARGE_FILE)
    .union(FeatureRoCompatSet::BTREE_DIR)
    .union(FeatureRoCompatSet::HUGE_FILE)
    .union(FeatureRoCompatSet::DIR_NLINK)
    .union(FeatureRoCompatSet::EXTRA_ISIZE);

/// Validates that the volume's feature set is acceptable for the given mount
/// flavor.
///
/// The class-wide checks (incompatible features within `INCOMPAT_SUPP`,
/// read-only compatible features within `RO_COMPAT_SUPP`, `FILETYPE`
/// required) run during superblock decoding; this adds the per-flavor rule.
/// An `ext2` mount only accepts true ext2-format volumes, mirroring Linux's
/// `IS_EXT2_SB`: a volume with extent mapping or a journal is an ext3/ext4
/// image and must be mounted as `ext4` instead.
pub(super) fn check_flavor(sb: &SuperBlock, flavor: MountFlavor) -> Result<()> {
    match flavor {
        MountFlavor::Ext4 => Ok(()),
        MountFlavor::Ext2 => {
            if sb.feature_incompat().contains(FeatureIncompatSet::EXTENTS) {
                return_errno_with_message!(Errno::EINVAL, "volume uses extents; mount it as ext4");
            }
            if sb.feature_compat().contains(FeatureCompatSet::HAS_JOURNAL) {
                return_errno_with_message!(Errno::EINVAL, "volume has a journal; mount it as ext4");
            }
            Ok(())
        }
    }
}
