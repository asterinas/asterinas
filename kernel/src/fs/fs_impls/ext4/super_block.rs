// SPDX-License-Identifier: MPL-2.0

//! On-disk ext4 superblock parsing and the validated in-memory representation.
//!
//! The ext4 superblock shares its first 264 bytes of layout with ext2; the
//! ext4-specific fields live in the trailing reserved area. The group
//! descriptor size (`s_desc_size`) is parsed here so `flex_bg` images mount
//! (their bitmaps/tables come from the descriptor getters, so relocating them
//! needs no geometry change), and the `64BIT` high halves of the block counts
//! (`s_blocks_count_hi` / `s_free_blocks_count_hi`) are spliced at the parse
//! boundary so `> 2^32`-block volumes count correctly. The metadata-checksum
//! fields are parsed and, for a `metadata_csum` volume, the superblock checksum
//! is verified at this boundary.

use super::{
    checksum::{self, FsCsumSeed},
    feature::{
        FeatureCompatSet, FeatureIncompatSet, FeatureRoCompatSet, INCOMPAT_SUPP, RO_COMPAT_SUPP,
    },
    inode::RawInode,
    prelude::*,
};

/// Magic signature (`s_magic`).
pub(super) const MAGIC_NUM: u16 = 0xef53;

/// The main superblock is located at byte 1024 from the start of the device.
pub(super) const SUPER_BLOCK_OFFSET: usize = 1024;

const SUPER_BLOCK_SIZE: usize = 1024;

/// Classic group-descriptor size in bytes, and the size forced when the `64BIT`
/// feature is absent (Linux `EXT4_MIN_DESC_SIZE`).
const MIN_DESC_SIZE: u16 = 32;

/// Smallest group descriptor a `64BIT` volume may declare (Linux
/// `EXT4_MIN_DESC_SIZE_64BIT`). With `64BIT` set, `s_desc_size` must be at
/// least this — a smaller value contradicts the wide on-disk GDT stride.
const MIN_DESC_SIZE_64BIT: u16 = 64;

/// Widest group descriptor the `64BIT` feature defines (Linux
/// `EXT4_MAX_DESC_SIZE`); the 64-byte descriptor carries the high halves and
/// per-group checksums.
const MAX_DESC_SIZE: u16 = 64;

/// `s_flags` bit recording that the on-disk htree hashes were computed treating
/// name bytes as signed `char` (`EXT2_FLAGS_SIGNED_HASH`). Present here for
/// documentation of the two-bit signedness encoding; the reader only needs the
/// unsigned bit, defaulting to signed when neither is set.
#[expect(dead_code)]
const EXT2_FLAGS_SIGNED_HASH: u32 = 0x1;

/// `s_flags` bit recording that the on-disk htree hashes were computed treating
/// name bytes as unsigned `char` (`EXT2_FLAGS_UNSIGNED_HASH`); selects the
/// `*_UNSIGNED` hash variants in `dx_probe`.
const EXT2_FLAGS_UNSIGNED_HASH: u32 = 0x2;

/// Validated, Rust-typed in-memory representation of the ext4 superblock.
///
/// Counts that the `64BIT` feature widens are stored as `u64`, and the parse
/// boundary splices in their high halves for a `64BIT` volume (the high halves
/// are zero on a 32-bit volume).
#[derive(Clone, Copy, Debug)]
pub(super) struct SuperBlock {
    inodes_count: u32,
    blocks_count: u64,
    free_blocks_count: u64,
    free_inodes_count: u32,
    first_data_block: Ext4Bid,
    block_size: usize,
    nr_blocks_per_group: u32,
    /// Number of block groups, computed once at parse from the geometry above
    /// (rounding up the last partial group).
    nr_block_groups: u32,
    nr_inodes_per_group: u32,
    // The fields below are decoded from the raw superblock and consumed by the
    // read, statfs, and checksum-verify paths. A few not yet read by any path
    // (e.g. `rev_level`, `state`) still carry an `#[expect(dead_code)]` marker on
    // their accessor.
    nr_inode_table_blocks_per_group: u32,
    inode_size: usize,
    /// Effective group-descriptor size in bytes (32 or 64), already resolved
    /// from the raw `s_desc_size` sentinel by [`parse_desc_size`].
    desc_size: u16,
    first_ino: u32,
    rev_level: RevLevel,
    state: FsState,
    feature_compat: FeatureCompatSet,
    feature_incompat: FeatureIncompatSet,
    feature_ro_compat: FeatureRoCompatSet,
    uuid: [u8; 16],
    last_orphan: Option<Ext4Ino>,
    reserved_blocks_count: u32,
    /// `s_reserved_gdt_blocks` widened once at parse; consumed only by
    /// [`Self::metadata_overhead`] (an image built with `^resize_inode` has none).
    reserved_gdt_blocks: u32,
    /// `s_hash_seed`: the four seed words feeding the htree name hash. Consumed
    /// by `dx_probe` (P6d) to reproduce the hashes the entries were indexed by.
    hash_seed: [u32; 4],
    /// Whether the on-disk htree hashes treat name bytes as unsigned `char`
    /// (`s_flags & EXT2_FLAGS_UNSIGNED_HASH`); picks the `*_UNSIGNED` hash
    /// variant. Defaults to signed (`false`) when neither signedness bit is set.
    hash_unsigned: bool,
}

impl TryFrom<RawSuperBlock> for SuperBlock {
    type Error = Error;

    fn try_from(sb: RawSuperBlock) -> Result<Self> {
        if sb.magic != MAGIC_NUM {
            return_errno_with_message!(Errno::EINVAL, "bad ext4 magic number");
        }

        // Only the 4 KiB block size the page-cache model assumes (page index ==
        // logical block) is supported.
        if sb.log_block_size != 2 {
            return_errno_with_message!(Errno::EINVAL, "unsupported block size (4 KiB only)");
        }
        if sb.log_frag_size != sb.log_block_size {
            return_errno_with_message!(Errno::EINVAL, "invalid fragment size");
        }
        let block_size = BLOCK_SIZE;

        let state = FsState::from_bits_truncate(sb.state);

        let errors_behavior = ErrorsBehavior::try_from(sb.errors)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid errors behavior"))?;
        if errors_behavior != ErrorsBehavior::Continue {
            return_errno_with_message!(Errno::EINVAL, "unsupported errors behavior");
        }

        let creator_os = OsId::try_from(sb.creator_os)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid creator os"))?;
        if creator_os != OsId::Linux {
            return_errno_with_message!(Errno::EINVAL, "unsupported creator os");
        }

        let rev_level = RevLevel::try_from(sb.rev_level)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid revision level"))?;
        let (first_ino, inode_size) = match rev_level {
            RevLevel::GoodOld => (11, 128usize),
            RevLevel::Dynamic => {
                let inode_size = sb.inode_size as usize;
                // Must divide the 4 KiB block (power of two, not exceeding it) so no
                // inode straddles a block boundary.
                if inode_size > block_size || !inode_size.is_power_of_two() {
                    return_errno_with_message!(Errno::EINVAL, "invalid inode size");
                }
                (sb.first_ino, inode_size)
            }
        };

        // The inode decode reads a fixed `size_of::<RawInode>()`
        // (256-byte) slot, and the metadata-csum path checks exactly that raw
        // slot. This 256-centric port therefore admits exactly 256-byte inodes:
        // a narrower legacy slot would make the fixed read window cross inode
        // boundaries, while a wider slot would make checksum verification slice
        // past the fixed `RawInode` buffer.
        // Checked after the rev-level match so it also covers the `GoodOld` path,
        // which forces 128 and skips the `Dynamic` size validation above.
        if inode_size != size_of::<RawInode>() {
            return_errno_with_message!(
                Errno::EINVAL,
                "unsupported inode size: this read-only port handles only 256-byte inodes"
            );
        }

        // Reject any incompatible feature this phase cannot honor, rather than
        // silently misreading the volume.
        let unsupported_incompat = sb.feature_incompat & !INCOMPAT_SUPP.bits();
        if unsupported_incompat != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported incompatible feature");
        }
        let feature_incompat = FeatureIncompatSet::from_bits_truncate(sb.feature_incompat);
        if !feature_incompat.contains(FeatureIncompatSet::EXTENTS) {
            return_errno_with_message!(Errno::EINVAL, "ext4 image without the extents feature");
        }

        // Read-only fail-closed (dirty-volume policy): this mount carries no
        // journal-replay machinery, so a volume that still needs recovery must be
        // refused, not read verbatim as a possibly pre-crash inconsistent state.
        // The RECOVER incompat bit is already rejected above (dropped from
        // `INCOMPAT_SUPP`); a pending orphan list (`s_last_orphan != 0`) is the
        // other half of `needs_recovery` and is refused here.
        if sb.last_orphan != 0 {
            return_errno_with_message!(
                Errno::EROFS,
                "dirty ext4 volume needs recovery; refusing read-only mount"
            );
        }
        let feature_compat = FeatureCompatSet::from_bits_truncate(sb.feature_compat);

        // Resolve `s_desc_size` at this boundary into the effective descriptor
        // size the group-descriptor decoder strides by: `EXT4_DESC_SIZE =
        // has_64bit ? s_desc_size : 32`. With `64BIT` now supported, a 64-byte
        // descriptor is admitted and decoded wide by `BlockGroup::read_desc`.
        let desc_size = parse_desc_size(
            sb.desc_size,
            feature_incompat.contains(FeatureIncompatSet::IS_64BIT),
        )?;

        // A ro_compat feature outside `RO_COMPAT_SUPP` (e.g. `GDT_CSUM` or
        // `BIGALLOC`) may carry on-disk layout or semantics this reader cannot
        // interpret, so mounting the volume would risk misreading it. Refuse
        // the mount with `EROFS` (Linux `ext4_setup_super` refuses `MS_RDWR`
        // the same way). Checked on the raw bits so bits unknown to
        // `FeatureRoCompatSet` are caught too.
        if sb.feature_ro_compat & !RO_COMPAT_SUPP.bits() != 0 {
            return_errno_with_message!(
                Errno::EROFS,
                "unsupported read-only-compatible feature; refusing mount"
            );
        }
        let feature_ro_compat = FeatureRoCompatSet::from_bits_truncate(sb.feature_ro_compat);

        // Verify the superblock's own checksum at the parse boundary (Linux
        // `ext4_superblock_csum_verify`). `METADATA_CSUM` is in `RO_COMPAT_SUPP`,
        // so a checksummed volume passes the `EROFS` gate above and reaches here
        // on every mount.
        if feature_ro_compat.contains(FeatureRoCompatSet::METADATA_CSUM) {
            Self::verify_superblock_checksum(&sb)?;
        }

        let nr_inodes_per_group = sb.inodes_per_group;
        let nr_blocks_per_group = sb.blocks_per_group;
        if nr_inodes_per_group == 0 || nr_blocks_per_group == 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid group sizes");
        }

        let inodes_per_block = (block_size / inode_size) as u32;
        let max_bits_per_group = (block_size as u32) * 8;
        if nr_inodes_per_group < inodes_per_block || nr_inodes_per_group > max_bits_per_group {
            return_errno_with_message!(Errno::EINVAL, "invalid inodes per group");
        }
        if nr_blocks_per_group > max_bits_per_group {
            return_errno_with_message!(Errno::EINVAL, "blocks per group is too large");
        }

        let nr_inode_table_blocks_per_group = nr_inodes_per_group / inodes_per_block;
        if nr_blocks_per_group <= nr_inode_table_blocks_per_group + 3 {
            return_errno_with_message!(Errno::EINVAL, "blocks per group is too small");
        }

        // Splice the 64-bit block counts at this one parse boundary: the high
        // halves are honored only with the `64BIT` feature, and a non-64bit image
        // carrying a non-zero high half is malformed (rejected inside the helper).
        let is_64bit = feature_incompat.contains(FeatureIncompatSet::IS_64BIT);
        let blocks_count = splice_count_hi(sb.blocks_count, sb.blocks_count_hi, is_64bit)?;
        let free_blocks_count =
            splice_count_hi(sb.free_blocks_count, sb.free_blocks_count_hi, is_64bit)?;

        let first_data_block = sb.first_data_block as u64;
        if blocks_count <= first_data_block + 1 {
            return_errno_with_message!(Errno::EINVAL, "invalid blocks count");
        }
        let nr_block_groups =
            (blocks_count - first_data_block - 1) / nr_blocks_per_group as u64 + 1;
        let nr_block_groups = u32::try_from(nr_block_groups)
            .map_err(|_| Error::with_message(Errno::EINVAL, "block group count exceeds 32 bits"))?;

        let max_inodes = nr_block_groups as u64 * nr_inodes_per_group as u64;
        let min_inodes = (nr_block_groups as u64 - 1) * nr_inodes_per_group as u64;
        let inodes_count = sb.inodes_count as u64;
        if inodes_count <= min_inodes || inodes_count > max_inodes {
            return_errno_with_message!(Errno::EINVAL, "invalid inodes count");
        }
        if free_blocks_count > blocks_count {
            return_errno_with_message!(Errno::EINVAL, "free blocks count exceeds blocks count");
        }
        if sb.free_inodes_count > sb.inodes_count {
            return_errno_with_message!(Errno::EINVAL, "free inodes count exceeds inodes count");
        }

        Ok(Self {
            inodes_count: sb.inodes_count,
            blocks_count,
            free_blocks_count,
            free_inodes_count: sb.free_inodes_count,
            first_data_block,
            block_size,
            nr_blocks_per_group,
            nr_block_groups,
            nr_inodes_per_group,
            nr_inode_table_blocks_per_group,
            inode_size,
            desc_size,
            first_ino,
            rev_level,
            state,
            feature_compat,
            feature_incompat,
            feature_ro_compat,
            uuid: sb.uuid,
            // `0 = empty` is the on-disk convention; in memory the head is an
            // `Option` and the sentinel stops at this parse boundary.
            last_orphan: (sb.last_orphan != 0).then_some(sb.last_orphan),
            reserved_blocks_count: sb.reserved_blocks_count,
            reserved_gdt_blocks: u32::from(sb.reserved_gdt_blocks),
            hash_seed: sb.hash_seed,
            // Neither signedness bit set means signed `char` (the historical
            // default); only `UNSIGNED_HASH` flips it (Linux `ext4_fill_super`).
            hash_unsigned: sb.flags & EXT2_FLAGS_UNSIGNED_HASH != 0,
        })
    }
}

impl SuperBlock {
    pub(super) const fn block_size(&self) -> usize {
        self.block_size
    }

    pub(super) const fn inode_size(&self) -> usize {
        self.inode_size
    }

    /// Returns the effective group-descriptor size in bytes (`EXT4_DESC_SIZE`):
    /// the raw `s_desc_size` when the `64BIT` feature is set, else the classic
    /// 32. Drives the GDT stride and the 32-vs-64-byte descriptor decode in
    /// [`BlockGroup::load`](super::block_group::BlockGroup).
    pub(super) const fn desc_size(&self) -> u16 {
        self.desc_size
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn first_ino(&self) -> u32 {
        self.first_ino
    }

    pub(super) const fn nr_inodes_per_group(&self) -> u32 {
        self.nr_inodes_per_group
    }

    pub(super) const fn nr_blocks_per_group(&self) -> u32 {
        self.nr_blocks_per_group
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn nr_inode_table_blocks_per_group(&self) -> u32 {
        self.nr_inode_table_blocks_per_group
    }

    pub(super) const fn first_data_block(&self) -> Ext4Bid {
        self.first_data_block
    }

    pub(super) const fn total_inodes(&self) -> u32 {
        self.inodes_count
    }

    pub(super) const fn total_blocks(&self) -> u64 {
        self.blocks_count
    }

    pub(super) const fn free_blocks_count(&self) -> u64 {
        self.free_blocks_count
    }

    pub(super) const fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    /// Returns the number of block groups (computed once at parse, rounding up
    /// the last partial group).
    pub(super) const fn nr_block_groups(&self) -> u32 {
        self.nr_block_groups
    }

    /// Returns the number of inodes stored per block.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn inodes_per_block(&self) -> u32 {
        (self.block_size / self.inode_size) as u32
    }

    #[expect(dead_code)]
    pub(super) const fn rev_level(&self) -> RevLevel {
        self.rev_level
    }

    #[expect(dead_code)]
    pub(super) const fn state(&self) -> FsState {
        self.state
    }

    pub(super) const fn uuid(&self) -> &[u8; 16] {
        &self.uuid
    }

    /// Returns whether this volume carries crc32c metadata checksums
    /// (`metadata_csum`). When false, every checksum compute/verify site is a
    /// no-op, so a checksum-free image is handled byte-for-byte as in Phases 1–5.
    pub(super) fn has_metadata_csum(&self) -> bool {
        self.feature_ro_compat
            .contains(FeatureRoCompatSet::METADATA_CSUM)
    }

    /// Returns the per-filesystem checksum seed feeding the group-descriptor,
    /// inode, bitmap, directory, and extent checksums: `crc32c(!0, uuid)`. (A
    /// `csum_seed`-incompat volume, which would override this with an on-disk
    /// seed word, is refused at the incompat gate, so the UUID is authoritative.)
    /// The superblock's own checksum does not use this seed.
    pub(super) fn metadata_csum_seed(&self) -> FsCsumSeed {
        FsCsumSeed::new(checksum::crc32c(!0, &self.uuid))
    }

    /// Computes the crc32c of `raw`'s first [`S_CHECKSUM_OFFSET`] bytes: the
    /// superblock's own checksum. Seeded with `!0` — the superblock, unlike group
    /// descriptors and inodes, does not use the per-filesystem seed (Linux
    /// `ext4_superblock_csum`). The covered range stops short of `s_checksum`, so
    /// the result does not depend on the field's current value; a writer stores
    /// it straight back.
    pub(super) fn superblock_checksum(raw: &RawSuperBlock) -> u32 {
        checksum::crc32c(!0, &raw.as_bytes()[..S_CHECKSUM_OFFSET])
    }

    /// Verifies `raw`'s stored `s_checksum` (and that `s_checksum_type` names
    /// crc32c), for a `metadata_csum` volume at the parse boundary.
    pub(super) fn verify_superblock_checksum(raw: &RawSuperBlock) -> Result<()> {
        if raw.as_bytes()[S_CHECKSUM_TYPE_OFFSET] != CHECKSUM_TYPE_CRC32C {
            return_errno_with_message!(Errno::EUCLEAN, "superblock checksum type is not crc32c");
        }
        if raw.checksum != Self::superblock_checksum(raw) {
            return_errno_with_message!(Errno::EUCLEAN, "bad superblock checksum");
        }
        Ok(())
    }

    /// Returns the blocks reserved for privileged processes
    /// (`s_r_blocks_count`); `statfs` subtracts them from `bfree` to report
    /// `bavail`.
    pub(super) const fn reserved_blocks_count(&self) -> u32 {
        self.reserved_blocks_count
    }

    /// Returns whether block group `group` carries a copy of the superblock and
    /// group-descriptor table (Linux `ext4_bg_has_super`).
    ///
    /// Without `sparse_super` every group has one. With it, only groups 0 and 1
    /// and the odd powers of 3, 5, and 7 do; the rest carry data only. The
    /// unsupported `sparse_super2` layout (not in `RO_COMPAT_SUPP`) is refused at
    /// the mount gate, so this mirrors the sparse/dense split alone.
    fn bg_has_super(&self, group: u32) -> bool {
        if group <= 1 {
            return true;
        }
        if !self
            .feature_ro_compat
            .contains(FeatureRoCompatSet::SPARSE_SUPER)
        {
            return true;
        }
        if group.is_multiple_of(2) {
            return false;
        }
        is_power_of(group, 3) || is_power_of(group, 5) || is_power_of(group, 7)
    }

    /// Computes the on-disk metadata overhead in blocks, excluding the journal
    /// (Linux `ext4_calculate_overhead`, non-`bigalloc` path; `bigalloc` is not
    /// in `RO_COMPAT_SUPP`, so every mounted volume takes this path).
    ///
    /// The sum is: the blocks before `first_data_block`, plus per group the block
    /// bitmap, inode bitmap, and inode-table blocks — and, in each group that
    /// carries a superblock/GDT copy, the superblock block, the group-descriptor
    /// blocks, and the reserved GDT-growth blocks. `flex_bg` only relocates these
    /// within a flex group without changing their count, so the geometry sum is
    /// unchanged. `statfs` reports `f_blocks = total_blocks - overhead`,
    /// so unprivileged `df` sees usable capacity rather than the raw device size.
    pub(super) fn metadata_overhead(&self) -> u64 {
        // Number of group descriptors that fit in one block (`EXT4_DESC_PER_BLOCK`),
        // hence the count of primary GDT blocks. `meta_bg` (not in `INCOMPAT_SUPP`)
        // would spread the GDT differently, so its absence keeps this contiguous.
        let desc_per_block = u64::try_from(self.block_size / usize::from(self.desc_size)).unwrap();
        let gdt_blocks = u64::from(self.nr_block_groups).div_ceil(desc_per_block);

        let per_group_data = u64::from(self.nr_inode_table_blocks_per_group) + 2;
        let per_super_group = 1 + gdt_blocks + u64::from(self.reserved_gdt_blocks);

        let groups_with_super = u64::try_from(
            (0..self.nr_block_groups)
                .filter(|&group| self.bg_has_super(group))
                .count(),
        )
        .unwrap();

        self.first_data_block
            + per_group_data * u64::from(self.nr_block_groups)
            + per_super_group * groups_with_super
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn feature_incompat(&self) -> FeatureIncompatSet {
        self.feature_incompat
    }

    #[expect(dead_code)]
    pub(super) const fn feature_ro_compat(&self) -> FeatureRoCompatSet {
        self.feature_ro_compat
    }

    /// Returns the four-word htree name-hash seed (`s_hash_seed`).
    pub(super) fn hash_seed(&self) -> &[u32; 4] {
        &self.hash_seed
    }

    /// Returns whether the on-disk htree hashes treat name bytes as unsigned
    /// `char` (`s_flags & EXT2_FLAGS_UNSIGNED_HASH`); `dx_probe` adds this to the
    /// root's hash version to pick the matching variant.
    pub(super) fn hash_unsigned(&self) -> bool {
        self.hash_unsigned
    }

    /// Returns whether the volume carries htree directory indexes
    /// (`COMPAT_DIR_INDEX`); a directory can only be probed as an htree when set.
    pub(super) fn has_dir_index(&self) -> bool {
        self.feature_compat.contains(FeatureCompatSet::DIR_INDEX)
    }

    /// Returns whether the volume still needs journal replay (the `RECOVER` bit
    /// is set or an orphan list is pending). A read-only mount refuses such a
    /// volume at parse time rather than reading a possibly pre-crash state.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn needs_recovery(&self) -> bool {
        self.feature_incompat.contains(FeatureIncompatSet::RECOVER) || self.last_orphan.is_some()
    }
}

/// Resolves and validates the raw `s_desc_size` into the effective
/// group-descriptor size in bytes.
///
/// Mirrors Linux ext4 `super.c`: the on-disk field is honored only with the
/// `64BIT` feature (`EXT4_DESC_SIZE = has_64bit ? s_desc_size : 32`); a `0` on
/// disk means the classic 32-byte descriptor either way. When honored the size
/// must be a power of two within `[MIN_DESC_SIZE, MAX_DESC_SIZE]`.
///
/// The classic-layout branch is deliberately strict: without `64BIT` the
/// descriptor is 32 bytes, so a raw value other than the unset sentinel or the
/// classic size would be silently misread, and we reject it instead.
fn parse_desc_size(raw: u16, has_64bit: bool) -> Result<u16> {
    if !has_64bit {
        // Without the 64bit feature `s_desc_size` is not authoritative: `0`
        // (unset) or the classic `32` both mean 32-byte descriptors, and any
        // other value is a malformed superblock. The sentinel `0` stops here.
        if raw != 0 && raw != MIN_DESC_SIZE {
            return_errno_with_message!(
                Errno::EINVAL,
                "group descriptor size set without the 64bit feature"
            );
        }
        return Ok(MIN_DESC_SIZE);
    }

    // With 64bit, `s_desc_size` is authoritative and MUST describe a 64-bit
    // descriptor. Mirror Linux `super.c` (EXT4_MIN_DESC_SIZE_64BIT): it does NOT
    // fold `0` to a default here and rejects anything below 64. Otherwise a
    // 64bit image whose `s_desc_size` disagrees with its physical 64-byte GDT
    // stride would mount and read every group's descriptor at the wrong offset —
    // the refused-mount → silent-cross-linked-corruption the red-line forbids.
    if raw < MIN_DESC_SIZE_64BIT || raw > MAX_DESC_SIZE || !raw.is_power_of_two() {
        return_errno_with_message!(Errno::EINVAL, "invalid 64bit group descriptor size");
    }
    // `read_desc` strides by this size and decodes the 64-byte layout's high
    // halves; it stays in lockstep with `IS_64BIT ∈ INCOMPAT_SUPP`.
    Ok(raw)
}

/// Whether `value` is a positive integer power of `base` (`value == base^k`,
/// `k >= 1`), mirroring Linux `test_root`. Sparse-super backups live in groups
/// that are powers of 3, 5, or 7, so [`SuperBlock::bg_has_super`] tests each base.
fn is_power_of(value: u32, base: u32) -> bool {
    let mut remaining = value;
    while remaining > base {
        if !remaining.is_multiple_of(base) {
            return false;
        }
        remaining /= base;
    }
    remaining == base
}

/// Splices a superblock 64-bit block count's low and high halves into a `u64`.
///
/// The one read-side boundary for `s_{blocks,free_blocks}_count{,_hi}`
/// (rust_rules #3). `(lo as u64) | ((hi as u64) << 32)` is lossless. The high
/// half is honored only with the `64BIT` feature; a non-64bit image with a
/// non-zero high half is malformed — that field is defined only under `64BIT` —
/// and is rejected rather than silently folded in.
fn splice_count_hi(lo: u32, hi: u32, has_64bit: bool) -> Result<u64> {
    if !has_64bit {
        if hi != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "64-bit count high half set without the 64bit feature"
            );
        }
        return Ok(lo as u64);
    }
    Ok((lo as u64) | ((hi as u64) << 32))
}

/// The ext4 revision level (`s_rev_level`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum RevLevel {
    /// Original format with a fixed 128-byte inode.
    GoodOld = 0,
    /// V2 format with a dynamic inode size (ext4 uses this).
    Dynamic = 1,
}

bitflags! {
    /// Filesystem state (`s_state`).
    pub(super) struct FsState: u16 {
        /// Unmounted cleanly.
        const VALID = 1 << 0;
        /// Errors detected.
        const ERROR = 1 << 1;
        /// Orphan inodes are being recovered.
        const ORPHAN = 1 << 2;
    }
}

/// Action taken when an error is detected (`s_errors`).
#[repr(u16)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, TryFromInt)]
pub(super) enum ErrorsBehavior {
    /// Continues execution.
    #[default]
    Continue = 1,
    /// Remounts the filesystem read-only.
    RemountReadonly = 2,
    /// Panics.
    Panic = 3,
}

/// OS that created the filesystem (`s_creator_os`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum OsId {
    Linux = 0,
    Hurd = 1,
    Masix = 2,
    FreeBSD = 3,
    Lites = 4,
}

const_assert!(size_of::<RawSuperBlock>() == SUPER_BLOCK_SIZE);

/// The on-disk superblock structure (exactly 1024 bytes).
///
/// Convert to `SuperBlock` via `TryFrom` for the validated representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawSuperBlock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub reserved_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    /// The number to left-shift 1024 by to obtain the block size.
    pub log_block_size: u32,
    /// The number to left-shift 1024 by to obtain the fragment size.
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: UnixTime,
    pub wtime: UnixTime,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub min_rev_level: u16,
    pub last_check_time: UnixTime,
    pub check_interval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub default_reserved_uid: u16,
    pub default_reserved_gid: u16,
    pub first_ino: u32,
    pub inode_size: u16,
    pub block_group_idx: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: Str16,
    pub last_mounted_dir: Str64,
    pub algorithm_usage_bitmap: u32,
    pub prealloc_file_blocks: u8,
    pub prealloc_dir_blocks: u8,
    /// `s_reserved_gdt_blocks` (0xCE): blocks reserved after the group-descriptor
    /// table so the GDT can grow on an online resize. They are filesystem
    /// overhead in every group that carries a superblock/GDT copy, so
    /// [`SuperBlock::metadata_overhead`] counts them there (Linux `count_overhead`).
    pub reserved_gdt_blocks: u16,
    pub journal_uuid: [u8; 16],
    pub journal_ino: u32,
    pub journal_dev: u32,
    pub last_orphan: u32,
    pub hash_seed: [u32; 4],
    pub def_hash_version: u8,
    pub(super) reserved_char_pad: u8,
    /// `s_desc_size`: on-disk group-descriptor size in bytes (offset 0xFE).
    /// Honored only with the `64BIT` feature; `0` means the classic 32-byte
    /// descriptor. Validated at the parse boundary by [`parse_desc_size`].
    pub(super) desc_size: u16,
    pub default_mount_opts: u32,
    pub first_meta_bg: u32,
    /// `s_mkfs_time` (0x108): filesystem creation time.
    pub mkfs_time: UnixTime,
    /// `s_jnl_blocks` (0x10C): backup of the journal inode's block map.
    pub jnl_blocks: [u32; 17],
    /// `s_blocks_count_hi` (0x150): high 32 bits of the total block count.
    /// Honored only with the `64BIT` feature; spliced with `blocks_count` at the
    /// parse boundary ([`SuperBlock::try_from`]).
    pub blocks_count_hi: u32,
    /// `s_r_blocks_count_hi` (0x154): high 32 bits of the reserved block count.
    /// Named for correct field placement; `reserved_blocks_count` stays 32-bit
    /// (a >2^32-block reserve is beyond the supported geometry).
    pub r_blocks_count_hi: u32,
    /// `s_free_blocks_count_hi` (0x158): high 32 bits of the free block count.
    /// Honored only with the `64BIT` feature; spliced with `free_blocks_count` at
    /// the parse boundary.
    pub free_blocks_count_hi: u32,
    /// `s_min_extra_isize` (0x15C, u16) + `s_want_extra_isize` (0x15E, u16),
    /// carved out only to place `flags` at the correct offset; not consumed.
    pub(super) extra_isize_hints: u32,
    /// `s_flags` (0x160): miscellaneous filesystem flags. The two low bits record
    /// which `char` signedness the on-disk htree hashes were computed with
    /// ([`EXT2_FLAGS_SIGNED_HASH`] / [`EXT2_FLAGS_UNSIGNED_HASH`]); `dx_probe`
    /// reads them to pick the matching hash variant.
    pub flags: u32,
    pub(super) reserved: Reserved,
    /// `s_checksum` (0x3FC): crc32c of the superblock over its first
    /// [`S_CHECKSUM_OFFSET`] bytes — the last word of the 1024-byte block.
    /// Meaningful only with `metadata_csum`; computed by
    /// [`SuperBlock::superblock_checksum`] and verified at the parse boundary.
    pub checksum: u32,
}

/// Byte offset of `s_checksum` in the on-disk superblock (0x3FC): the crc32c
/// covers exactly `[0, S_CHECKSUM_OFFSET)`, stopping short of the field itself.
const S_CHECKSUM_OFFSET: usize = 0x3FC;

/// Byte offset of `s_checksum_type` (0x175); `metadata_csum` requires it to name
/// crc32c ([`CHECKSUM_TYPE_CRC32C`]).
const S_CHECKSUM_TYPE_OFFSET: usize = 0x175;

/// The only `s_checksum_type` ext4 defines: crc32c.
const CHECKSUM_TYPE_CRC32C: u8 = 1;

/// Reserved padding filling the on-disk superblock up to `s_checksum` at 0x3FC.
/// In ext4 this region also holds the checksum-seed and mount-option fields; the
/// checksum seed is derived from the UUID instead (a `csum_seed`-incompat volume
/// is refused at the feature gate, so the on-disk seed word is never consulted).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct Reserved([u32; 166]);

impl Default for Reserved {
    fn default() -> Self {
        Self([0u32; 166])
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    /// Builds a minimal-feature (extent + filetype) raw superblock for a small
    /// 4 KiB-block image with the given geometry.
    fn minimal_raw(
        blocks_count: u32,
        blocks_per_group: u32,
        inodes_per_group: u32,
    ) -> RawSuperBlock {
        let nr_groups = (blocks_count - 1) / blocks_per_group + 1;
        RawSuperBlock {
            inodes_count: nr_groups * inodes_per_group,
            blocks_count,
            free_blocks_count: 0,
            free_inodes_count: 0,
            first_data_block: 0,
            log_block_size: 2,
            log_frag_size: 2,
            blocks_per_group,
            frags_per_group: blocks_per_group,
            inodes_per_group,
            magic: MAGIC_NUM,
            state: FsState::VALID.bits(),
            errors: ErrorsBehavior::Continue as u16,
            creator_os: OsId::Linux as u32,
            rev_level: RevLevel::Dynamic as u32,
            first_ino: 11,
            inode_size: 256,
            feature_incompat: (FeatureIncompatSet::FILETYPE | FeatureIncompatSet::EXTENTS).bits(),
            feature_ro_compat: FeatureRoCompatSet::SPARSE_SUPER.bits(),
            ..Default::default()
        }
    }

    #[ktest]
    fn parse_minimal_superblock() {
        let raw = minimal_raw(2048, 2048, 256);
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.block_size(), 4096);
        assert_eq!(sb.inode_size(), 256);
        assert_eq!(sb.first_ino(), 11);
        assert_eq!(sb.inodes_per_block(), 16);
        assert_eq!(sb.nr_inode_table_blocks_per_group(), 16);
        assert_eq!(sb.nr_block_groups(), 1);
        assert!(sb.feature_incompat().contains(FeatureIncompatSet::EXTENTS));
        assert!(!sb.needs_recovery());
    }

    #[ktest]
    fn reject_bad_magic() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.magic = 0x1234;
        assert!(SuperBlock::try_from(raw).is_err());
    }

    #[ktest]
    fn reject_non_4k_block() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.log_block_size = 0; // 1 KiB
        assert!(SuperBlock::try_from(raw).is_err());
    }

    /// A legacy 128-byte inode is refused at admission. The inode decode reads a
    /// fixed 256-byte `RawInode`, so this 256-centric port admits exactly that
    /// slot width rather than misreading old-style inode tables.
    #[ktest]
    fn reject_inode_size_below_256() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.inode_size = 128;
        assert!(SuperBlock::try_from(raw).is_err());
    }

    /// Wider inode slots are refused too: the read path currently fetches only
    /// the 256-byte `RawInode` and metadata-csum verification must not slice
    /// beyond that fixed buffer.
    #[ktest]
    fn reject_inode_size_not_256() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.inode_size = 512;
        let Err(err) = SuperBlock::try_from(raw) else {
            panic!("wider inode size must be refused at mount");
        };
        assert_eq!(err.error(), Errno::EINVAL);
    }

    #[ktest]
    fn reject_unsupported_incompat() {
        // `MMP` is a genuine incompatible feature this implementation does not
        // support (it is not in `INCOMPAT_SUPP`), so it must refuse the mount.
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.feature_incompat |= FeatureIncompatSet::MMP.bits();
        assert!(SuperBlock::try_from(raw).is_err());
    }

    /// A ro_compat feature outside `RO_COMPAT_SUPP` must refuse the mount with
    /// `EROFS` — its semantics are ones this reader cannot interpret (Linux
    /// `ext4_setup_super` parity).
    #[ktest]
    fn reject_unsupported_ro_compat() {
        let mut raw = minimal_raw(2048, 2048, 256);
        // `GDT_CSUM` (legacy crc16 group-descriptor checksums) is a known but
        // unimplemented ro_compat feature — mutually exclusive with the
        // `METADATA_CSUM` we do support — so it is refused with `EROFS`.
        raw.feature_ro_compat |= FeatureRoCompatSet::GDT_CSUM.bits();
        let Err(err) = SuperBlock::try_from(raw) else {
            panic!("unsupported ro_compat must be refused at mount");
        };
        assert_eq!(err.error(), Errno::EROFS);

        // A bit unknown to `FeatureRoCompatSet` entirely (raw-bits check).
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.feature_ro_compat |= 1 << 12; // RO_COMPAT_READONLY
        assert!(SuperBlock::try_from(raw).is_err());
    }

    /// Fail-closed dirty-volume policy: a superblock with a pending orphan list
    /// (`s_last_orphan != 0`) is `needs_recovery`, which this read-only,
    /// journal-less mount cannot perform. It is refused with `EROFS` rather than
    /// silently exposing the crash-time inconsistent state behind it — the same
    /// fail-closed stance as dropping `RECOVER` from `INCOMPAT_SUPP`.
    #[ktest]
    fn reject_dirty_volume_pending_orphan() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.last_orphan = 5;
        let Err(err) = SuperBlock::try_from(raw) else {
            panic!("a volume needing orphan recovery must not mount");
        };
        assert_eq!(err.error(), Errno::EROFS);
    }

    /// `superblock_checksum` covers exactly the first `S_CHECKSUM_OFFSET` bytes,
    /// so recomputing after only the `s_checksum` field changes is stable, and a
    /// change anywhere in the covered range flips it.
    #[ktest]
    fn superblock_checksum_covers_body_not_field() {
        let mut raw = minimal_raw(2048, 2048, 256);
        let csum = SuperBlock::superblock_checksum(&raw);

        // Storing the checksum (the excluded last word) does not change it.
        raw.checksum = csum;
        assert_eq!(SuperBlock::superblock_checksum(&raw), csum);
        raw.checksum = 0xDEAD_BEEF;
        assert_eq!(SuperBlock::superblock_checksum(&raw), csum);

        // A change inside the covered body does change it.
        raw.blocks_count += 1;
        assert_ne!(SuperBlock::superblock_checksum(&raw), csum);
    }

    /// `verify_superblock_checksum` accepts a correctly stamped superblock and
    /// rejects a corrupted one or a non-crc32c checksum type with `EUCLEAN`.
    #[ktest]
    fn verify_superblock_checksum_round_trip() {
        let mut raw = minimal_raw(2048, 2048, 256);
        // `s_checksum_type` (0x175) must name crc32c (== 1).
        raw.as_mut_bytes()[S_CHECKSUM_TYPE_OFFSET] = CHECKSUM_TYPE_CRC32C;
        raw.checksum = SuperBlock::superblock_checksum(&raw);
        SuperBlock::verify_superblock_checksum(&raw).unwrap();

        // Corrupt the stored checksum.
        let mut bad = raw;
        bad.checksum ^= 1;
        assert_eq!(
            SuperBlock::verify_superblock_checksum(&bad)
                .unwrap_err()
                .error(),
            Errno::EUCLEAN
        );

        // Wrong checksum type is rejected even with a matching value.
        let mut wrong_type = raw;
        wrong_type.as_mut_bytes()[S_CHECKSUM_TYPE_OFFSET] = 0;
        wrong_type.checksum = SuperBlock::superblock_checksum(&wrong_type);
        assert_eq!(
            SuperBlock::verify_superblock_checksum(&wrong_type)
                .unwrap_err()
                .error(),
            Errno::EUCLEAN
        );
    }

    #[ktest]
    fn reject_without_extents() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.feature_incompat = FeatureIncompatSet::FILETYPE.bits();
        assert!(SuperBlock::try_from(raw).is_err());
    }

    #[ktest]
    fn multi_group_geometry() {
        // 3 full groups of 2048 blocks: blocks_count = 3*2048.
        let raw = minimal_raw(3 * 2048, 2048, 256);
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.nr_block_groups(), 3);
        assert_eq!(sb.total_blocks(), 3 * 2048);
    }

    /// `test_root`/`is_power_of` picks out the sparse-super backup groups.
    #[ktest]
    fn is_power_of_matches_test_root() {
        assert!(is_power_of(3, 3) && is_power_of(9, 3) && is_power_of(81, 3));
        assert!(is_power_of(5, 5) && is_power_of(125, 5));
        assert!(is_power_of(7, 7) && is_power_of(49, 7));
        // Not powers: composites and the identity/zero cases.
        assert!(!is_power_of(15, 3) && !is_power_of(15, 5));
        assert!(!is_power_of(1, 3) && !is_power_of(6, 3));
    }

    /// The `ext4_calculate_overhead` (non-bigalloc) geometry sum. `minimal_raw`
    /// sets `sparse_super`, `first_data_block == 0`, inode size 256 (16 inodes
    /// per 4 KiB block → 16 inode-table blocks per group), and no reserved GDT.
    #[ktest]
    fn metadata_overhead_sparse_super() {
        // 10 groups of 2048 blocks. GDT fits in one block (10 <= 4096/32). The
        // groups carrying a superblock/GDT copy are 0, 1 and the odd powers of
        // 3/5/7 below 10 — i.e. 3, 5, 7, 9 — so six of the ten.
        let raw = minimal_raw(10 * 2048, 2048, 256);
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.nr_block_groups(), 10);
        // 10 * (16 inode-table + 2 bitmaps) + 6 * (1 super + 1 GDT + 0 reserved).
        assert_eq!(sb.metadata_overhead(), 10 * (16 + 2) + 6 * (1 + 1));
    }

    /// Reserved GDT-growth blocks add to every superblock-bearing group.
    #[ktest]
    fn metadata_overhead_reserved_gdt() {
        let mut raw = minimal_raw(10 * 2048, 2048, 256);
        raw.reserved_gdt_blocks = 100;
        let sb = SuperBlock::try_from(raw).unwrap();
        // The six super-bearing groups each also carry 100 reserved GDT blocks.
        assert_eq!(sb.metadata_overhead(), 10 * (16 + 2) + 6 * (1 + 1 + 100));
    }

    /// Without `sparse_super`, every group carries a superblock/GDT copy.
    #[ktest]
    fn metadata_overhead_dense_super() {
        let mut raw = minimal_raw(10 * 2048, 2048, 256);
        raw.feature_ro_compat = 0; // clear SPARSE_SUPER
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.metadata_overhead(), 10 * (16 + 2) + 10 * (1 + 1));
    }

    /// A GDT spanning more than one block: with 130 groups the primary GDT needs
    /// `ceil(130 / (4096/32)) = 2` blocks, charged to each super-bearing group.
    #[ktest]
    fn metadata_overhead_multi_gdt_block() {
        let raw = minimal_raw(130 * 2048, 2048, 256);
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.nr_block_groups(), 130);
        // Super-bearing groups: 0, 1 plus powers of 3 (3,9,27,81), 5 (5,25,125),
        // 7 (7,49) below 130 — eleven groups; each carries 1 super + 2 GDT blocks.
        assert_eq!(sb.metadata_overhead(), 130 * (16 + 2) + 11 * (1 + 2));
    }

    /// A `0` on-disk `s_desc_size` (the mkfs default for non-64bit images)
    /// resolves to the classic 32-byte descriptor.
    #[ktest]
    fn desc_size_defaults_to_classic() {
        let raw = minimal_raw(2048, 2048, 256);
        assert_eq!(raw.desc_size, 0);
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.desc_size(), MIN_DESC_SIZE);
    }

    /// An explicit `s_desc_size == 32` resolves to 32.
    #[ktest]
    fn desc_size_explicit_classic() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.desc_size = MIN_DESC_SIZE;
        let sb = SuperBlock::try_from(raw).unwrap();
        assert_eq!(sb.desc_size(), MIN_DESC_SIZE);
    }

    /// Without `64BIT`, any `s_desc_size` other than 0 or 32 is rejected rather
    /// than silently misread by the 32-byte decoder.
    #[ktest]
    fn reject_out_of_range_desc_size() {
        for bad in [16u16, 33, 48, 64, 128] {
            let mut raw = minimal_raw(2048, 2048, 256);
            raw.desc_size = bad;
            let Err(err) = SuperBlock::try_from(raw) else {
                panic!("desc_size {bad} must be rejected without 64bit");
            };
            assert_eq!(err.error(), Errno::EINVAL);
        }
    }

    /// The `EXT4_DESC_SIZE = has_64bit ? s_desc_size : 32` gating, exercised at
    /// the parse boundary directly (the mount-level `IS_64BIT` gate rejects the
    /// feature earlier today, so the 64bit branch is only reachable here).
    #[ktest]
    fn parse_desc_size_gating() {
        // Without 64BIT: 0 and 32 resolve to 32; anything else is rejected.
        assert_eq!(parse_desc_size(0, false).unwrap(), MIN_DESC_SIZE);
        assert_eq!(parse_desc_size(32, false).unwrap(), MIN_DESC_SIZE);
        assert!(parse_desc_size(64, false).is_err());
        assert!(parse_desc_size(48, false).is_err());

        // With 64BIT: `s_desc_size` is authoritative and must be >= 64 (Linux
        // EXT4_MIN_DESC_SIZE_64BIT); 0 and 32 are REJECTED — they would
        // contradict the wide 64-byte GDT stride — as are non-power-of-two and
        // out-of-range. Only 64 is admitted.
        assert_eq!(parse_desc_size(MAX_DESC_SIZE, true).unwrap(), MAX_DESC_SIZE);
        assert!(parse_desc_size(0, true).is_err());
        assert!(parse_desc_size(32, true).is_err());
        assert!(parse_desc_size(48, true).is_err());
        assert!(parse_desc_size(16, true).is_err());
        assert!(parse_desc_size(128, true).is_err());
    }

    /// A `flex_bg` image mounts now that `FLEX_BG` is in `INCOMPAT_SUPP` (the
    /// read side already locates bitmaps/tables through the descriptor getters).
    #[ktest]
    fn accept_flex_bg() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.feature_incompat |= FeatureIncompatSet::FLEX_BG.bits();
        let sb = SuperBlock::try_from(raw).unwrap();
        assert!(sb.feature_incompat().contains(FeatureIncompatSet::FLEX_BG));
    }

    /// The `s_*_count_hi` splice boundary: the high half is honored only under
    /// `64BIT`, and a non-64bit high half is rejected rather than folded in. A
    /// genuine `> 2^32`-block image cannot be built here (the fixture's `u32`
    /// `inodes_count` cannot express that geometry), so the wide decode itself is
    /// verified at this boundary.
    #[ktest]
    fn splice_count_hi_gating() {
        // Without 64BIT: the low half passes through; any high half is rejected.
        assert_eq!(splice_count_hi(0x1234, 0, false).unwrap(), 0x1234);
        assert!(splice_count_hi(0, 1, false).is_err());
        assert!(splice_count_hi(5, 7, false).is_err());

        // With 64BIT: `lo | (hi << 32)`, lossless.
        assert_eq!(splice_count_hi(0xDEAD_BEEF, 0, true).unwrap(), 0xDEAD_BEEF);
        assert_eq!(splice_count_hi(0, 1, true).unwrap(), 1u64 << 32);
        assert_eq!(
            splice_count_hi(0x8000_0001, 2, true).unwrap(),
            (2u64 << 32) | 0x8000_0001
        );
    }

    /// A non-64bit image carrying a non-zero `s_blocks_count_hi` or
    /// `s_free_blocks_count_hi` is malformed and rejected at parse time.
    #[ktest]
    fn reject_high_count_without_64bit() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.blocks_count_hi = 1;
        assert_eq!(
            SuperBlock::try_from(raw).unwrap_err().error(),
            Errno::EINVAL
        );

        let mut raw = minimal_raw(2048, 2048, 256);
        raw.free_blocks_count_hi = 1;
        assert_eq!(
            SuperBlock::try_from(raw).unwrap_err().error(),
            Errno::EINVAL
        );
    }

    /// A `64BIT` image (feature bit + `s_desc_size == 64`) mounts, records the
    /// wide descriptor size, and round-trips the (here `< 2^32`) free-block count
    /// through the u64 splice.
    #[ktest]
    fn accept_64bit_image() {
        let mut raw = minimal_raw(2048, 2048, 256);
        raw.feature_incompat |= FeatureIncompatSet::IS_64BIT.bits();
        raw.desc_size = MAX_DESC_SIZE;
        raw.free_blocks_count = 500;
        // High halves zero (this small image fits in 32 bits).
        let sb = SuperBlock::try_from(raw).unwrap();
        assert!(sb.feature_incompat().contains(FeatureIncompatSet::IS_64BIT));
        assert_eq!(sb.desc_size(), MAX_DESC_SIZE);
        assert_eq!(sb.free_blocks_count(), 500u64);
        assert_eq!(sb.total_blocks(), 2048u64);
    }
}
