// SPDX-License-Identifier: MPL-2.0

//! Read-side htree (`dir_index`) index traversal, ported from Linux 6.6
//! `fs/ext4/namei.c` (`dx_probe`).
//!
//! An htree directory keeps a hash index in its first data blocks: a `dx_root`
//! in logical block 0 and, for larger directories, one level of
//! `dx_node` index blocks, each an array of `{ hash, block }` entries sorted by
//! hash. To locate the leaf block that may hold a name, hash the name and, at
//! each level, binary-search the entries for the last one whose hash does not
//! exceed the target, then descend into the block it points at. The leaf block
//! is then scanned linearly by the caller.
//!
//! This module only *decodes and descends* the index (path C of P6d): it never
//! writes a dx block, so the per-block checksum trailer (`dx_tail`) is neither
//! consulted nor produced here, matching the verify-on-read deferrals elsewhere
//! in P6. Insert/build/flatten of the index is a later task.
//!
//! The byte layout is decoded field-by-field at the block boundary (never by
//! overlaying a struct), because the count/limit of the first entry slot is
//! overlaid onto the hash word of `entries[0]` — a struct view would misread it.

use super::{super::super::prelude::*, hash, hash::DX_HASH_TEA};

/// The filesystem-wide inputs to the htree name hash, snapshotted from the
/// superblock so a directory lookup can probe the index without re-reading it.
/// Built by the caller only for a `dir_index` volume; a directory that carries
/// the `INDEX` flag is then probed, and any miss falls back to a linear scan.
#[derive(Clone, Copy, Debug)]
pub(super) struct DxCtx {
    /// The `s_hash_seed` words.
    pub seed: [u32; 4],
    /// Whether name bytes hash as unsigned `char` (`s_flags` bit).
    pub unsigned: bool,
}

/// Size of one on-disk `struct dx_entry` (`{ __le32 hash, __le32 block }`).
const DX_ENTRY_SIZE: usize = 8;

/// Byte offset of `entries[]` within a `dx_root` block: two 12-byte fake dirents
/// (`.` and `..`, each an 8-byte header plus a 4-byte name field) followed by the
/// 8-byte `dx_root_info`. Production derives the offset from the parsed
/// `info_length` (`DX_ROOT_INFO_OFF + info_length`); this named constant is the
/// spec value the ktest fixtures build against.
#[cfg_attr(not(ktest), expect(dead_code))]
const DX_ROOT_ENTRIES_OFF: usize = 32;

/// Byte offset of `entries[]` within a `dx_node` block: a single 8-byte fake
/// dirent header (`inode == 0`, `rec_len == blocksize`).
const DX_NODE_ENTRIES_OFF: usize = 8;

/// Byte offset of `dx_root_info` within a `dx_root` block (after the two
/// 12-byte fake dirents).
const DX_ROOT_INFO_OFF: usize = 24;

/// The mandated `dx_root_info.info_length` (`sizeof(struct dx_root_info)`).
const DX_ROOT_INFO_LEN: u8 = 8;

/// Largest `indirect_levels` an htree root may declare without the (unsupported)
/// `largedir` feature: 0 (root points straight at leaf blocks) or 1 (root →
/// one `dx_node` level → leaves). Linux `dx_probe` refuses more than
/// `ext4_dir_htree_level() - 1` == 1 for a non-`largedir` volume; a deeper tree
/// is refused here with `EUCLEAN`.
const MAX_INDIRECT_LEVELS: u8 = 1;

/// The low 28 bits of a `dx_entry.block` hold the logical block number; the top
/// four are reserved (Linux `dx_get_block`).
const DX_BLOCK_MASK: u32 = 0x0fff_ffff;

/// Reads a little-endian `u32` at byte offset `off` in `buf`.
///
/// `buf` is always a full [`BLOCK_SIZE`] directory block and every caller keeps
/// `off + 4` within it, so the slice is in range.
fn read_le32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

/// Reads a little-endian `u16` at byte offset `off` in `buf`.
fn read_le16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

/// A validated view of one index block's `dx_entry` array (a `dx_root`'s or a
/// `dx_node`'s), decoded once at `off` within a full-block buffer.
///
/// `entries[0]`'s hash word is overlaid by `{ limit, count }` (Linux
/// `dx_countlimit`), so the real index entries are `1..count`; entry 0 carries
/// only the hash-0 catch-all block. Construction validates the `{limit,count}`
/// header and that the declared array fits the block, so every later
/// [`hash`](Self::hash)/[`block`](Self::block) read is in bounds (a malformed
/// index is rejected here rather than read off the end).
struct DxEntries<'a> {
    buf: &'a [u8],
    off: usize,
    count: usize,
}

impl<'a> DxEntries<'a> {
    fn parse(buf: &'a [u8], off: usize) -> Result<Self> {
        let limit = read_le16(buf, off) as usize;
        let count = read_le16(buf, off + 2) as usize;
        if count < 1 || count > limit || off + limit * DX_ENTRY_SIZE > BLOCK_SIZE {
            return_errno_with_message!(Errno::EUCLEAN, "htree: dx entry count out of range");
        }
        Ok(Self { buf, off, count })
    }

    fn count(&self) -> usize {
        self.count
    }

    /// The hash key of entry `i` (`dx_get_hash`); entry 0's slot is the overlaid
    /// `{limit,count}`, so callers treat it as the implicit-0 floor.
    fn hash(&self, i: usize) -> u32 {
        read_le32(self.buf, self.off + i * DX_ENTRY_SIZE)
    }

    /// The child block of entry `i` (`dx_get_block`), masked to 28 bits.
    fn block(&self, i: usize) -> Ext4Bid {
        Ext4Bid::from(read_le32(self.buf, self.off + i * DX_ENTRY_SIZE + 4))
            & Ext4Bid::from(DX_BLOCK_MASK)
    }
}

/// The validated `dx_root_info` header of an htree directory's root block.
#[derive(Debug)]
pub(super) struct DxRootInfo {
    /// Stored hash version (`DX_HASH_*`), before the unsigned-`char` adjustment.
    hash_version: u8,
    /// `info_length`; validated to equal [`DX_ROOT_INFO_LEN`].
    info_length: u8,
    /// Number of index levels below the root (0 or 1; a deeper tree needs the
    /// unsupported `largedir` feature and is rejected — see
    /// [`MAX_INDIRECT_LEVELS`]).
    indirect_levels: u8,
}

/// Parses and validates the `dx_root_info` header from a directory's logical
/// block 0 (Linux `dx_probe` prologue).
///
/// Rejects (`EUCLEAN`) a header whose `info_length` is not the mandated 8, whose
/// `unused_flags` low bit is set (an on-disk hash flag we do not implement), or
/// whose declared depth exceeds [`MAX_INDIRECT_LEVELS`]. The hash version is not
/// validated here: an unrecognised or `siphash` version is handled downstream by
/// [`super::hash::ext4fs_dirhash`] returning `None`, which the caller turns into a linear-scan
/// fallback.
pub(super) fn parse_dx_root(block0: &[u8]) -> Result<DxRootInfo> {
    // dx_root_info: reserved_zero(4) hash_version(1) info_length(1)
    // indirect_levels(1) unused_flags(1).
    let hash_version = block0[DX_ROOT_INFO_OFF + 4];
    let info_length = block0[DX_ROOT_INFO_OFF + 5];
    let indirect_levels = block0[DX_ROOT_INFO_OFF + 6];
    let unused_flags = block0[DX_ROOT_INFO_OFF + 7];

    if info_length != DX_ROOT_INFO_LEN {
        return_errno_with_message!(Errno::EUCLEAN, "htree: unexpected dx_root_info length");
    }
    if unused_flags & 1 != 0 {
        return_errno_with_message!(Errno::EUCLEAN, "htree: unsupported hash flags");
    }
    if indirect_levels > MAX_INDIRECT_LEVELS {
        return_errno_with_message!(Errno::EUCLEAN, "htree: index depth exceeds supported level");
    }
    Ok(DxRootInfo {
        hash_version,
        info_length,
        indirect_levels,
    })
}

/// Descends the htree index to the single leaf block that could hold `name`,
/// returning its logical block number (Linux `dx_probe`).
///
/// `read_block` fetches a directory logical block through the page cache (the
/// caller supplies it so this stays a pure, testable traversal over the on-disk
/// bytes). `hash_seed` and `hash_unsigned` come from the superblock and, with the
/// root's stored hash version, fix which hash the entries were indexed by.
///
/// Returns `Ok(None)` when the directory's hash version is one we cannot compute
/// (e.g. `siphash`); the caller then falls back to a linear scan. Returns
/// `EUCLEAN` on a malformed index (bad header, out-of-range entry count, or a
/// block that points back at an ancestor — a cycle).
pub(super) fn dx_lookup_leaf(
    read_block: impl Fn(Ext4Bid) -> Result<[u8; BLOCK_SIZE]>,
    name: &[u8],
    hash_seed: &[u32; 4],
    hash_unsigned: bool,
) -> Result<Option<Ext4Bid>> {
    let root_block = read_block(0)?;
    let root = parse_dx_root(&root_block)?;

    // Effective hash version: the `*_UNSIGNED` variants sit +3 above their signed
    // twins, applied only to the three byte-hash versions (Linux `dx_probe`:
    // `hinfo->hash_version += s_hash_unsigned`).
    let version = if hash_unsigned && root.hash_version <= DX_HASH_TEA {
        root.hash_version + 3
    } else {
        root.hash_version
    };
    let Some(dirhash) = hash::ext4fs_dirhash(name, version, hash_seed) else {
        // Unsupported hash (e.g. siphash): let the caller scan linearly.
        return Ok(None);
    };
    let target = dirhash.hash;

    let indirect = root.indirect_levels;
    // Blocks visited on the path from the root, indexed by level, for the cycle
    // guard. `blocks[0]` is the root's own block (0); at most one entry per level
    // and `indirect <= MAX_INDIRECT_LEVELS` (1), so the root + one `dx_node`
    // level fit in three slots with room to spare.
    let mut blocks: [Ext4Bid; 3] = [0; 3];
    let mut level: u8 = 0;
    // Root `entries[]` follow `dx_root_info` (Linux `&root->info + info_length`);
    // `info_length` was validated to 8, so this is [`DX_ROOT_ENTRIES_OFF`].
    let mut entries_off = DX_ROOT_INFO_OFF + root.info_length as usize;
    let mut block_buf = root_block;

    loop {
        let entries = DxEntries::parse(&block_buf, entries_off)?;

        // Binary search entries[1..count] for the last entry whose hash does not
        // exceed the target; entry 0 (hash 0, the catch-all) is the floor, so
        // `at` is 0 when every real entry sorts above the target.
        let mut p: usize = 1;
        let mut q: usize = entries.count() - 1;
        while p <= q {
            let m = p + (q - p) / 2;
            if entries.hash(m) > target {
                q = m - 1;
            } else {
                p = m + 1;
            }
        }
        let block = entries.block(p - 1);

        // A block that reappears on the path back to the root is a cycle.
        if blocks[..=(level as usize)].contains(&block) {
            return_errno_with_message!(Errno::EUCLEAN, "htree: index cycle");
        }

        // At the last index level `block` is the leaf to scan (Linux
        // `if (++level > indirect) return frame`).
        if level == indirect {
            return Ok(Some(block));
        }
        level += 1;
        blocks[level as usize] = block;
        block_buf = read_block(block)?;
        entries_off = DX_NODE_ENTRIES_OFF;
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    // Explicit `Result` shadows the two glob imports (kernel + ostd preludes),
    // fixing the closure return type to the kernel `Result` `dx_lookup_leaf` uses.
    use super::{
        super::hash::{DX_HASH_HALF_MD4, ext4fs_dirhash},
        Result, *,
    };

    /// Seed and known hashes reused from the `hash.rs` reference vectors (signed
    /// half-MD4, the mkfs default): `ext4fs_dirhash(b"file0", HALF_MD4, seed)`.
    const MD4_SEED: [u32; 4] = [0xaec4_c740, 0x834c_e862, 0x09b5_3288, 0x34dc_d3db];
    const FILE0_HASH: u32 = 0x1c3d_2670;
    const FILE3_HASH: u32 = 0x4f04_13ba;

    /// Writes a `{ limit, count }` header plus the catch-all block and index
    /// entries into `blk` at `entries_off`. `entries[0]` is `(_, catch_all_block)`
    /// (its hash slot is the count/limit overlay); the rest are `(hash, block)`.
    fn write_entries(blk: &mut [u8; BLOCK_SIZE], entries_off: usize, entries: &[(u32, u32)]) {
        let count = entries.len() as u16;
        let limit = count + 10; // headroom; only `count` entries are read
        blk[entries_off..entries_off + 2].copy_from_slice(&limit.to_le_bytes());
        blk[entries_off + 2..entries_off + 4].copy_from_slice(&count.to_le_bytes());
        // entries[0]: catch-all block only (hash word is the overlay).
        blk[entries_off + 4..entries_off + 8].copy_from_slice(&entries[0].1.to_le_bytes());
        for (i, &(hash, block)) in entries.iter().enumerate().skip(1) {
            let off = entries_off + i * DX_ENTRY_SIZE;
            blk[off..off + 4].copy_from_slice(&hash.to_le_bytes());
            blk[off + 4..off + 8].copy_from_slice(&block.to_le_bytes());
        }
    }

    fn build_dx_root(
        hash_version: u8,
        indirect_levels: u8,
        entries: &[(u32, u32)],
    ) -> [u8; BLOCK_SIZE] {
        let mut blk = [0u8; BLOCK_SIZE];
        blk[DX_ROOT_INFO_OFF + 4] = hash_version;
        blk[DX_ROOT_INFO_OFF + 5] = DX_ROOT_INFO_LEN;
        blk[DX_ROOT_INFO_OFF + 6] = indirect_levels;
        blk[DX_ROOT_INFO_OFF + 7] = 0; // unused_flags
        write_entries(&mut blk, DX_ROOT_ENTRIES_OFF, entries);
        blk
    }

    fn build_dx_node(entries: &[(u32, u32)]) -> [u8; BLOCK_SIZE] {
        let mut blk = [0u8; BLOCK_SIZE];
        write_entries(&mut blk, DX_NODE_ENTRIES_OFF, entries);
        blk
    }

    /// The reference hashes this test file pins really are what the hasher
    /// produces, so the entry layouts below bracket the right value.
    #[ktest]
    fn known_hash_vectors() {
        assert_eq!(
            ext4fs_dirhash(b"file0", DX_HASH_HALF_MD4, &MD4_SEED)
                .unwrap()
                .hash,
            FILE0_HASH
        );
        assert_eq!(
            ext4fs_dirhash(b"file3", DX_HASH_HALF_MD4, &MD4_SEED)
                .unwrap()
                .hash,
            FILE3_HASH
        );
    }

    /// A target below every real entry lands on `entries[0]`'s catch-all block.
    #[ktest]
    fn descent_lands_on_catch_all() {
        // file0 hashes to 0x1c3d2670, below both real entries.
        let root = build_dx_root(
            DX_HASH_HALF_MD4,
            0,
            &[(0, 10), (0x2000_0000, 11), (0x3000_0000, 12)],
        );
        let leaf = dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap();
        assert_eq!(leaf, Some(10));
    }

    /// A target sitting on a middle entry descends to that entry's block; the
    /// entry immediately above (strictly greater hash) is not taken.
    #[ktest]
    fn descent_lands_on_middle_entry() {
        let root = build_dx_root(
            DX_HASH_HALF_MD4,
            0,
            &[
                (0, 10),
                (0x1000_0000, 11),
                (FILE0_HASH, 12),
                (0x3000_0000, 13),
            ],
        );
        let leaf = dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap();
        assert_eq!(leaf, Some(12));
    }

    /// A target above every entry lands on the last one.
    #[ktest]
    fn descent_lands_on_last_entry() {
        // file3 hashes to 0x4f0413ba, above all three real entries.
        let root = build_dx_root(
            DX_HASH_HALF_MD4,
            0,
            &[
                (0, 10),
                (0x1000_0000, 11),
                (0x2000_0000, 12),
                (0x4000_0000, 13),
            ],
        );
        let leaf = dx_lookup_leaf(|b| single_block(b, &root), b"file3", &MD4_SEED, false).unwrap();
        assert_eq!(leaf, Some(13));
    }

    /// A one-level tree: the root's single catch-all points at a `dx_node`, whose
    /// entries are searched for the leaf.
    #[ktest]
    fn descent_two_levels() {
        let node = build_dx_node(&[
            (0, 100),
            (0x1000_0000, 101),
            (FILE0_HASH, 102),
            (0x3000_0000, 103),
        ]);
        // Root has one entry (catch-all) pointing at the node in block 5.
        let root = build_dx_root(DX_HASH_HALF_MD4, 1, &[(0, 5)]);
        let read = |b: Ext4Bid| match b {
            0 => Ok(root),
            5 => Ok(node),
            other => panic!("unexpected block read {other}"),
        };
        let leaf = dx_lookup_leaf(read, b"file0", &MD4_SEED, false).unwrap();
        assert_eq!(leaf, Some(102));
    }

    /// An unsupported hash version (siphash, 6) yields `None` so the caller can
    /// fall back to a linear scan, rather than an error.
    #[ktest]
    fn unsupported_hash_falls_back() {
        let root = build_dx_root(6 /* siphash */, 0, &[(0, 10), (0x2000_0000, 11)]);
        let leaf = dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap();
        assert_eq!(leaf, None);
    }

    /// A catch-all block that points back at the root (block 0) is a cycle.
    #[ktest]
    fn descent_rejects_cycle() {
        let root = build_dx_root(DX_HASH_HALF_MD4, 0, &[(0, 0)]);
        let err =
            dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap_err();
        assert_eq!(err.error(), Errno::EUCLEAN);
    }

    /// A zero entry count is rejected before any descent.
    #[ktest]
    fn descent_rejects_zero_count() {
        let mut root = build_dx_root(DX_HASH_HALF_MD4, 0, &[(0, 10), (0x2000_0000, 11)]);
        // Overwrite count (le16 at entries_off + 2) with 0.
        root[DX_ROOT_ENTRIES_OFF + 2..DX_ROOT_ENTRIES_OFF + 4].copy_from_slice(&0u16.to_le_bytes());
        let err =
            dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap_err();
        assert_eq!(err.error(), Errno::EUCLEAN);
    }

    /// A `limit` whose entry array cannot fit the block is rejected rather than
    /// read off the end — a crafted index must not panic the kernel.
    #[ktest]
    fn descent_rejects_oversize_limit() {
        let mut root = build_dx_root(DX_HASH_HALF_MD4, 0, &[(0, 10), (0x2000_0000, 11)]);
        // Overwrite limit (le16 at entries_off) with a value far larger than the
        // block can hold; count stays 2 (valid), so only the fit bound catches it.
        root[DX_ROOT_ENTRIES_OFF..DX_ROOT_ENTRIES_OFF + 2]
            .copy_from_slice(&60_000u16.to_le_bytes());
        let err =
            dx_lookup_leaf(|b| single_block(b, &root), b"file0", &MD4_SEED, false).unwrap_err();
        assert_eq!(err.error(), Errno::EUCLEAN);
    }

    /// `parse_dx_root` rejects a bad `info_length`, an unimplemented hash flag,
    /// and an over-deep tree.
    #[ktest]
    fn parse_dx_root_validation() {
        let mut root = build_dx_root(DX_HASH_HALF_MD4, 0, &[(0, 10)]);
        assert!(parse_dx_root(&root).is_ok());

        let mut bad = root;
        bad[DX_ROOT_INFO_OFF + 5] = 16; // info_length != 8
        assert_eq!(parse_dx_root(&bad).unwrap_err().error(), Errno::EUCLEAN);

        let mut bad = root;
        bad[DX_ROOT_INFO_OFF + 7] = 1; // unused_flags & 1
        assert_eq!(parse_dx_root(&bad).unwrap_err().error(), Errno::EUCLEAN);

        root[DX_ROOT_INFO_OFF + 6] = MAX_INDIRECT_LEVELS + 1; // too deep
        assert_eq!(parse_dx_root(&root).unwrap_err().error(), Errno::EUCLEAN);
    }

    /// Helper for the single-block (`indirect_levels == 0`) cases: only block 0 is
    /// ever read, the leaf block being returned without a read.
    fn single_block(b: Ext4Bid, root: &[u8; BLOCK_SIZE]) -> Result<[u8; BLOCK_SIZE]> {
        assert_eq!(b, 0, "only the root block should be read at depth 0");
        Ok(*root)
    }
}
