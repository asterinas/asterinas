// SPDX-License-Identifier: MPL-2.0

//! Ext4 block-group descriptors and the block-side read domain.
//!
//! Each block group owns its own block and inode bitmaps. At mount the parsed
//! descriptor and both bitmaps are loaded; for a group flagged `BLOCK_UNINIT` /
//! `INODE_UNINIT` the bitmap is reconstructed from the group's on-disk layout
//! rather than read from disk. This is a read-only mount, so the descriptor and
//! bitmaps are only read — never allocated against or written back.
//!
//! A per-group **inode cache** gives live inodes a stable identity (the same
//! `Arc<Inode>` for a given inode number) so concurrent readers share one
//! object.
//!
//! # Width invariant
//!
//! Group-relative bit indices and per-group counters are narrowed to `u16`
//! throughout this file. That leans on one parse-time invariant: a group holds
//! at most `block_size * 8 = 32768 < u16::MAX` blocks/inodes
//! (`SuperBlock::try_from` rejects larger `s_{blocks,inodes}_per_group`), and
//! counters never exceed the group capacity.
//!
//! # Locking
//!
//! `BlockGroup` uses two independent locks:
//!
//! - `metadata` — protects the group descriptor and the block and inode
//!   bitmaps.
//! - `inode_cache` — protects the per-group live inode map. Uses double-checked
//!   locking (read then promote to write on miss).

use core::fmt;

use super::{
    checksum::{self, FsCsumSeed},
    fs::Ext4,
    inode::{Inode, InodeDesc, RawInode},
    prelude::*,
    super_block::SuperBlock,
    utils,
};

/// `bg_flags` bit: the group's inode bitmap and inode table are uninitialized
/// (`EXT4_BG_INODE_UNINIT`). No inode has ever been allocated here; the on-disk
/// inode bitmap is not maintained and the inode table is not zeroed.
const BG_INODE_UNINIT: u16 = 0x0001;
/// `bg_flags` bit: the group's block bitmap is uninitialized
/// (`EXT4_BG_BLOCK_UNINIT`). No data block has ever been allocated here; the
/// on-disk block bitmap is not maintained and must be reconstructed from the
/// group layout (only the group's fixed metadata/backup overhead is in use).
const BG_BLOCK_UNINIT: u16 = 0x0002;

const_assert!(size_of::<RawBlockGroup>() == 32);

/// On-disk block-group descriptor, 32 bytes (without the `64BIT` high halves).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawBlockGroup {
    pub block_bitmap_lo: u32,
    pub inode_bitmap_lo: u32,
    pub inode_table_lo: u32,
    pub free_blocks_count_lo: u16,
    pub free_inodes_count_lo: u16,
    pub used_dirs_count_lo: u16,
    /// `bg_flags` (e.g. `INODE_UNINIT`, `BLOCK_UNINIT`, `INODE_ZEROED`).
    pub flags: u16,
    pub exclude_bitmap_lo: u32,
    pub block_bitmap_csum_lo: u16,
    pub inode_bitmap_csum_lo: u16,
    pub itable_unused_lo: u16,
    pub checksum: u16,
}

const_assert!(size_of::<RawBlockGroupHi>() == 32);

/// The 32-byte high-half tail of a `64BIT` group descriptor (`ext4_group_desc`
/// bytes 32..64). Present only when `s_desc_size == 64`; carries the block-number
/// high halves plus the per-group checksums the `metadata_csum` feature adds.
///
/// The per-group counter high halves (`*_count_hi`, `itable_unused_hi`) are
/// structurally zero in our geometry: a group holds at most `block_size * 8 =
/// 32768 < u16::MAX` blocks/inodes, so the counters never overflow their low
/// half. Only the block-number high halves can be non-zero (on volumes past
/// `2^32` blocks), and they are the only tail fields the decoder splices.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawBlockGroupHi {
    pub block_bitmap_hi: u32,
    pub inode_bitmap_hi: u32,
    pub inode_table_hi: u32,
    pub free_blocks_count_hi: u16,
    pub free_inodes_count_hi: u16,
    pub used_dirs_count_hi: u16,
    pub itable_unused_hi: u16,
    pub exclude_bitmap_hi: u32,
    pub block_bitmap_csum_hi: u16,
    pub inode_bitmap_csum_hi: u16,
    pub reserved: u32,
}

const_assert!(size_of::<RawBlockGroup64>() == 64);

/// On-disk 64-byte `64BIT` group descriptor: the classic 32-byte low half
/// ([`RawBlockGroup`]) followed by the 32-byte high-half tail
/// ([`RawBlockGroupHi`]). Read whole when `s_desc_size == 64`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawBlockGroup64 {
    pub lo: RawBlockGroup,
    pub hi: RawBlockGroupHi,
}

/// Validated, Rust-typed block-group descriptor.
///
/// Block numbers are `Ext4Bid` (`u64`) so the `64BIT` high halves slot in later
/// without widening; in Phase 2 they are the 32-bit low halves.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockGroupDesc {
    block_bitmap_bid: Ext4Bid,
    inode_bitmap_bid: Ext4Bid,
    inode_table_bid: Ext4Bid,
    free_blocks_count: u32,
    free_inodes_count: u32,
    used_dirs_count: u32,
    /// `bg_flags` — carries the `BLOCK_UNINIT`/`INODE_UNINIT` lazy-init bits. Kept
    /// so the block side can reconstruct an uninitialized bitmap on load.
    flags: u16,
}

impl BlockGroupDesc {
    /// Decodes a group descriptor from its raw low half and, for `64BIT` volumes,
    /// its high-half tail. This is the single parse-once boundary where the block
    /// numbers combine `lo | (hi << 32)` (rust_rules #3); no caller splices high
    /// halves itself, so a decode bug lives in exactly one place.
    ///
    /// The `(lo as u64) | ((hi as u64) << 32)` assembly is lossless: `lo` is
    /// `u32`, `hi` is `u32`, and their union fits `u64` with no bits dropped. The
    /// per-group counters are taken from the low half only — their high halves
    /// are structurally zero (see [`RawBlockGroupHi`]).
    fn from_raw(lo: &RawBlockGroup, hi: Option<&RawBlockGroupHi>) -> Self {
        let (block_bitmap_hi, inode_bitmap_hi, inode_table_hi) = match hi {
            Some(hi) => (hi.block_bitmap_hi, hi.inode_bitmap_hi, hi.inode_table_hi),
            None => (0, 0, 0),
        };
        Self {
            block_bitmap_bid: (lo.block_bitmap_lo as Ext4Bid)
                | ((block_bitmap_hi as Ext4Bid) << 32),
            inode_bitmap_bid: (lo.inode_bitmap_lo as Ext4Bid)
                | ((inode_bitmap_hi as Ext4Bid) << 32),
            inode_table_bid: (lo.inode_table_lo as Ext4Bid) | ((inode_table_hi as Ext4Bid) << 32),
            free_blocks_count: lo.free_blocks_count_lo as u32,
            free_inodes_count: lo.free_inodes_count_lo as u32,
            used_dirs_count: lo.used_dirs_count_lo as u32,
            flags: lo.flags,
        }
    }

    /// Computes the crc32c group-descriptor checksum (`metadata_csum`), low 16
    /// bits (Linux `ext4_group_desc_csum`). Seeded with the per-filesystem `seed`,
    /// then folded over the 0-based `group` number, the descriptor bytes up to
    /// `bg_checksum`, two zero bytes standing in for `bg_checksum` itself, and —
    /// for a 64-byte (`64BIT`) descriptor — the 32-byte high-half tail.
    fn group_desc_checksum(
        lo: &RawBlockGroup,
        hi: Option<&RawBlockGroupHi>,
        group: u32,
        seed: FsCsumSeed,
    ) -> u16 {
        // Byte offset of `bg_checksum` within the 32-byte low half.
        const BG_CHECKSUM_OFFSET: usize = 30;
        let mut crc = checksum::crc32c(seed.get(), &group.to_le_bytes());
        crc = checksum::crc32c(crc, &lo.as_bytes()[..BG_CHECKSUM_OFFSET]);
        crc = checksum::crc32c(crc, &[0u8, 0u8]); // bg_checksum, excluded from its own cover
        if let Some(hi) = hi {
            crc = checksum::crc32c(crc, hi.as_bytes());
        }
        (crc & 0xFFFF) as u16
    }

    /// Verifies a raw descriptor's stored `bg_checksum` for a `metadata_csum`
    /// volume, at the descriptor read boundary.
    fn verify_group_desc_checksum(
        lo: &RawBlockGroup,
        hi: Option<&RawBlockGroupHi>,
        group: u32,
        seed: FsCsumSeed,
    ) -> Result<()> {
        if lo.checksum != Self::group_desc_checksum(lo, hi, group, seed) {
            return_errno_with_message!(Errno::EUCLEAN, "bad group descriptor checksum");
        }
        Ok(())
    }

    /// Returns the starting block of this group's inode table.
    pub(super) const fn inode_table_bid(&self) -> Ext4Bid {
        self.inode_table_bid
    }

    pub(super) const fn block_bitmap_bid(&self) -> Ext4Bid {
        self.block_bitmap_bid
    }

    pub(super) const fn inode_bitmap_bid(&self) -> Ext4Bid {
        self.inode_bitmap_bid
    }

    pub(super) const fn free_blocks_count(&self) -> u32 {
        self.free_blocks_count
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn free_inodes_count(&self) -> u32 {
        self.free_inodes_count
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) const fn used_dirs_count(&self) -> u32 {
        self.used_dirs_count
    }

    /// Whether this group's on-disk block bitmap is uninitialized and must be
    /// reconstructed from the group layout (`EXT4_BG_BLOCK_UNINIT`).
    pub(super) const fn is_block_uninit(&self) -> bool {
        self.flags & BG_BLOCK_UNINIT != 0
    }

    /// Whether this group's inode bitmap and table are uninitialized
    /// (`EXT4_BG_INODE_UNINIT`).
    pub(super) const fn is_inode_uninit(&self) -> bool {
        self.flags & BG_INODE_UNINIT != 0
    }
}

/// One block group's metadata: the descriptor, the block bitmap, and the inode
/// bitmap.
///
/// All three members carry dirty tracking, though a read-only mount never marks
/// them dirty.
pub(super) struct BlockGroupMetadata {
    /// Group descriptor with dirty tracking.
    pub desc: Dirty<BlockGroupDesc>,
    /// Block bitmap cached in memory.
    pub block_bitmap: Dirty<IdBitmap>,
    /// Inode bitmap cached in memory.
    pub inode_bitmap: Dirty<IdBitmap>,
}

impl Debug for BlockGroupMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockGroupMetadata")
            .field("desc", &self.desc)
            .field("block_bitmap_dirty", &self.block_bitmap.is_dirty())
            .field("inode_bitmap_dirty", &self.inode_bitmap.is_dirty())
            .finish()
    }
}

/// A block group's read-side descriptor and bitmap cache.
///
/// Owns the cached block bitmap, inode bitmap, and group descriptor behind a
/// single lock; on a read-only mount these are only read, never allocated
/// against or written back.
pub(super) struct BlockGroup {
    /// Block group index (0-based).
    group_idx: usize,
    /// Group descriptor, block bitmap, and inode bitmap, protected by a single
    /// lock.
    metadata: RwMutex<BlockGroupMetadata>,
    /// Backing block device (shared with `Ext4` and other groups).
    block_device: Arc<dyn BlockDevice>,
    /// Cached geometry: inodes per group.
    nr_inodes_per_group: u32,
    /// Cached geometry: inode size in bytes.
    inode_size: usize,
    /// Cached geometry: filesystem block size in bytes.
    block_size: usize,
    /// The per-filesystem crc32c seed when `metadata_csum` is on, else `None`
    /// (checksums are a no-op). Cached from the superblock at load so the inode
    /// read path can verify without holding a `SuperBlock`.
    csum_seed: Option<FsCsumSeed>,
    /// Per-group live inode cache keyed by group-local inode index.
    ///
    /// Ext4 keeps this cache locally because the VFS layer does not provide a
    /// shared inode cache for filesystem implementations. It gives inodes a
    /// stable identity: every lookup of one inode number returns the same
    /// in-memory `Inode`, so concurrent readers share one object.
    inode_cache: RwMutex<BTreeMap<u16, Arc<Inode>>>,
}

impl Debug for BlockGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockGroup")
            .field("group_idx", &self.group_idx)
            .finish()
    }
}

impl BlockGroup {
    /// Loads a block group from the descriptor table.
    ///
    /// Reads and decodes the group's descriptor at `gdt_base_offset + group_idx *
    /// sb.desc_size()` (32 or 64 bytes wide per the `64BIT` feature), caches the
    /// group's geometry from `sb`, and loads the block bitmap.
    ///
    /// Loading is lenient: strict validation that the system-metadata blocks are
    /// marked allocated in the bitmap is deferred (the read-only fixtures carry
    /// an all-zero bitmap and must still mount).
    pub(super) fn load(
        device: Arc<dyn BlockDevice>,
        group_idx: usize,
        sb: &SuperBlock,
        gdt_base_offset: usize,
    ) -> Result<Self> {
        let desc_size = sb.desc_size();
        let desc_offset = gdt_base_offset + group_idx * desc_size as usize;
        let csum_seed = sb.has_metadata_csum().then(|| sb.metadata_csum_seed());
        let desc = Self::read_desc(
            device.as_ref(),
            desc_offset,
            desc_size,
            group_idx as u32,
            csum_seed,
        )?;

        // Cache geometry from `SuperBlock` at load time.
        let nr_blocks_per_group = sb.nr_blocks_per_group() as Ext4Bid;
        let first_block = sb.first_data_block() + (group_idx as Ext4Bid) * nr_blocks_per_group;
        let nr_block_groups = sb.nr_block_groups() as usize;
        let last_block = if group_idx == nr_block_groups - 1 {
            sb.total_blocks() - 1
        } else {
            first_block + nr_blocks_per_group - 1
        };
        let nr_inodes_per_group = sb.nr_inodes_per_group();
        let inode_size = sb.inode_size();
        let block_size = sb.block_size();

        // Load the block bitmap and the inode bitmap.
        let block_bitmap =
            Self::load_block_bitmap(device.as_ref(), first_block, last_block, &desc)?;
        let inode_bitmap = Self::load_inode_bitmap(device.as_ref(), nr_inodes_per_group, &desc)?;

        Ok(Self {
            group_idx,
            metadata: RwMutex::new(BlockGroupMetadata {
                desc: Dirty::new(desc),
                block_bitmap: Dirty::new(block_bitmap),
                inode_bitmap: Dirty::new(inode_bitmap),
            }),
            block_device: device,
            nr_inodes_per_group,
            inode_size,
            block_size,
            csum_seed,
            inode_cache: RwMutex::new(BTreeMap::new()),
        })
    }

    /// Reads and decodes this group's descriptor from `device` at `desc_offset`,
    /// sized by `desc_size`: the 64-byte `64BIT` layout — splicing the block-number
    /// high halves via [`BlockGroupDesc::from_raw`] — when `desc_size` is 64, else
    /// the classic 32-byte layout. The single decode path used by [`Self::load`],
    /// so the high-half splice lives at one boundary.
    /// `group` (0-based index) and `csum_seed` drive the `metadata_csum`
    /// verify-on-read: when `csum_seed` is `Some`, the stored `bg_checksum` is
    /// checked against a recomputation over this group's descriptor before the
    /// decode is trusted (`EUCLEAN` on mismatch); when `None` the descriptor is
    /// decoded as in Phases 1–5.
    fn read_desc(
        device: &dyn BlockDevice,
        desc_offset: usize,
        desc_size: u16,
        group: u32,
        csum_seed: Option<FsCsumSeed>,
    ) -> Result<BlockGroupDesc> {
        if desc_size as usize >= size_of::<RawBlockGroup64>() {
            let raw = device
                .read_val::<RawBlockGroup64>(desc_offset)
                .map_err(|_| Error::with_message(Errno::EIO, "failed to read group descriptor"))?;
            if let Some(seed) = csum_seed {
                BlockGroupDesc::verify_group_desc_checksum(&raw.lo, Some(&raw.hi), group, seed)?;
            }
            Ok(BlockGroupDesc::from_raw(&raw.lo, Some(&raw.hi)))
        } else {
            let raw = device
                .read_val::<RawBlockGroup>(desc_offset)
                .map_err(|_| Error::with_message(Errno::EIO, "failed to read group descriptor"))?;
            if let Some(seed) = csum_seed {
                BlockGroupDesc::verify_group_desc_checksum(&raw, None, group, seed)?;
            }
            Ok(BlockGroupDesc::from_raw(&raw, None))
        }
    }

    /// Returns the starting block of this group's inode table.
    pub(super) fn inode_table_bid(&self) -> Ext4Bid {
        self.metadata.read().desc.inode_table_bid()
    }

    /// Looks up an inode by inode number through this group's inode cache.
    ///
    /// Returns the same `Arc<Inode>` for repeated lookups of one inode number,
    /// so concurrent users share one in-memory inode (and one set of dirty
    /// state). The fast path hits the cache under the read lock; the slow path
    /// promotes to the write lock, re-checks (another thread may have inserted
    /// the inode in the gap), then loads the descriptor from disk and inserts it.
    pub(super) fn lookup_inode(&self, ino: Ext4Ino, fs: Weak<Ext4>) -> Result<Arc<Inode>> {
        let inode_idx = self.inode_idx_in_group(ino);

        // Fast path: cache hit under the read lock.
        if let Some(inode) = self.inode_cache.read().get(&inode_idx) {
            return Ok(inode.clone());
        }

        // Slow path: revalidate under the write lock, since another thread may
        // have inserted the inode between the read and write lock acquisition.
        let mut inode_cache = self.inode_cache.write();
        if let Some(inode) = inode_cache.get(&inode_idx) {
            return Ok(inode.clone());
        }

        let desc = self.read_inode_desc(ino)?;
        let type_ = desc.type_();
        let inode = Inode::new(ino, type_, Dirty::new(desc), self.group_idx, fs)?;
        inode_cache.insert(inode_idx, inode.clone());
        Ok(inode)
    }

    /// Loads and decodes an inode's on-disk descriptor from the inode table.
    ///
    /// The inode-table block is read through [`utils::read_metadata_block`], the
    /// single metadata-read funnel — a plain device read on this read-only,
    /// non-journaled mount, where the device is authoritative (the seam a
    /// journaling layer would later re-widen to consult its after-images).
    ///
    /// The fixed `size_of::<RawInode>()` (256-byte) read window never runs past
    /// the block: mount admission currently accepts exactly 256-byte inodes
    /// (`SuperBlock::try_from`), so each descriptor occupies one fixed slot and
    /// `off_in_block + size_of::<RawInode>()` stays within the metadata block
    /// that holds it.
    pub(super) fn read_inode_desc(&self, ino: Ext4Ino) -> Result<InodeDesc> {
        let idx_in_group = self.inode_idx_in_group(ino) as usize;
        let byte_off = idx_in_group * self.inode_size;
        let block_bid = self.inode_table_bid() + (byte_off / self.block_size) as Ext4Bid;
        let off_in_block = byte_off % self.block_size;
        let block = utils::read_metadata_block(self.block_device.as_ref(), block_bid)?;
        let raw = RawInode::from_bytes(&block[off_in_block..off_in_block + size_of::<RawInode>()]);
        if let Some(seed) = self.csum_seed {
            InodeDesc::verify_inode_checksum(
                &raw,
                seed.derive_inode(ino, raw.generation),
                self.inode_size,
            )?;
        }
        InodeDesc::try_from(&raw)
    }

    /// Returns the 0-based group-local inode index for `ino`.
    fn inode_idx_in_group(&self, ino: Ext4Ino) -> u16 {
        debug_assert!(ino > 0);
        debug_assert_eq!(
            ((ino - 1) / self.nr_inodes_per_group) as usize,
            self.group_idx
        );
        ((ino - 1) % self.nr_inodes_per_group) as u16
    }

    /// Loads the block bitmap for this group.
    fn load_block_bitmap(
        block_device: &dyn BlockDevice,
        first_block: Ext4Bid,
        last_block: Ext4Bid,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let group_size = (last_block - first_block + 1) as u32;
        let capacity = group_size as u16;
        debug_assert!(capacity as u32 == group_size && capacity <= IdBitmap::capacity());

        // A `BLOCK_UNINIT` group's on-disk block bitmap is not maintained, so the
        // raw block is meaningless (often all-zero) — trusting it would present
        // the group's backup superblock/GDT blocks as "free". Reconstruct the
        // bitmap from the group layout instead: such a group holds no data, so its
        // only used blocks are the fixed metadata/backup overhead at the group
        // start — a contiguous prefix whose length the descriptor's authoritative
        // free-block count gives us directly (Linux `ext4_init_block_bitmap`).
        if desc.is_block_uninit() {
            let overhead = group_size
                .checked_sub(desc.free_blocks_count())
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::EUCLEAN,
                        "uninit group free count exceeds group size",
                    )
                })?;
            let mut bitmap = IdBitmap::from_buf(vec![0u8; BLOCK_SIZE].into_boxed_slice(), capacity);
            if overhead > 0 {
                // `capacity == group_size >= overhead`, so this cannot fail.
                bitmap.alloc_consecutive(overhead as u16);
            }
            // Safety net for the prefix assumption: any of this group's own
            // metadata blocks that fall within the group must lie inside the
            // reconstructed prefix. If one sits beyond it the layout is not the
            // prefix we assumed — refuse rather than risk presenting that block
            // as free.
            for bid in [
                desc.block_bitmap_bid(),
                desc.inode_bitmap_bid(),
                desc.inode_table_bid(),
            ] {
                if (first_block..=last_block).contains(&bid)
                    && bid - first_block >= overhead as Ext4Bid
                {
                    return_errno_with_message!(
                        Errno::EUCLEAN,
                        "uninit group metadata block outside reconstructed prefix"
                    );
                }
            }
            return Ok(bitmap);
        }

        let bitmap_bid = desc.block_bitmap_bid();
        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read block bitmap");
        }

        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity))
    }

    /// Loads the inode bitmap for this group.
    ///
    /// The bitmap's logical capacity is the number of inodes per group, capped
    /// at the bitmap's physical capacity (a single block always holds at least
    /// as many bits as inodes a group can have).
    fn load_inode_bitmap(
        block_device: &dyn BlockDevice,
        nr_inodes_per_group: u32,
        desc: &BlockGroupDesc,
    ) -> Result<IdBitmap> {
        let capacity = nr_inodes_per_group.min(u32::from(IdBitmap::capacity())) as u16;

        // An `INODE_UNINIT` group has never had an inode allocated: its on-disk
        // inode bitmap is not maintained (raw content is meaningless). Present an
        // all-free bitmap rather than trusting the garbage, so any incidental
        // read (e.g. via a bogus inode number) sees a consistent empty group
        // instead of spurious allocated bits.
        if desc.is_inode_uninit() {
            return Ok(IdBitmap::from_buf(
                vec![0u8; BLOCK_SIZE].into_boxed_slice(),
                capacity,
            ));
        }

        let bitmap_bid = desc.inode_bitmap_bid();
        let mut buf = vec![0u8; BLOCK_SIZE];
        if block_device
            .read_bytes(Bid::new(bitmap_bid).to_offset(), &mut buf)
            .is_err()
        {
            return_errno_with_message!(Errno::EIO, "failed to read inode bitmap");
        }

        Ok(IdBitmap::from_buf(buf.into_boxed_slice(), capacity))
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{super::test_utils::Ext4MemoryDisk, *};

    /// A `BLOCK_UNINIT` group reconstructs its block bitmap from the layout — the
    /// leading `group_size - free_blocks_count` overhead blocks marked used, the
    /// rest free — and does NOT trust the (garbage) on-disk bitmap block. This
    /// prevents presenting a lazily-initialized group's backup metadata as
    /// "free" space.
    #[ktest]
    fn block_uninit_reconstructs_prefix_bitmap() {
        const GROUP_SIZE: u32 = 100;
        const OVERHEAD: u32 = 10;

        // Poison the on-disk block-bitmap block (bid 0) with all-ones: if the code
        // trusted it, every block would read as used and the asserts below fail.
        let disk = Ext4MemoryDisk::new(2);
        disk.segment()
            .write_bytes(0, &[0xFFu8; BLOCK_SIZE])
            .unwrap();

        let desc = BlockGroupDesc {
            block_bitmap_bid: 0,
            inode_bitmap_bid: 0,
            inode_table_bid: 0,
            free_blocks_count: GROUP_SIZE - OVERHEAD,
            free_inodes_count: 0,
            used_dirs_count: 0,
            flags: BG_BLOCK_UNINIT,
        };

        let bitmap =
            BlockGroup::load_block_bitmap(&disk, 0, (GROUP_SIZE - 1) as Ext4Bid, &desc).unwrap();

        for bit in 0..OVERHEAD as u16 {
            assert!(
                bitmap.is_allocated(bit),
                "overhead block {bit} must be used"
            );
        }
        for bit in OVERHEAD as u16..GROUP_SIZE as u16 {
            assert!(!bitmap.is_allocated(bit), "data block {bit} must be free");
        }
    }

    /// A metadata block sitting beyond the reconstructed prefix breaks the
    /// prefix assumption, so reconstruction refuses (fail-closed) rather than
    /// risk handing that block out.
    #[ktest]
    fn block_uninit_rejects_metadata_past_prefix() {
        let disk = Ext4MemoryDisk::new(2);
        let desc = BlockGroupDesc {
            block_bitmap_bid: 50, // in-range but past the 10-block prefix
            inode_bitmap_bid: 0,
            inode_table_bid: 0,
            free_blocks_count: 90,
            free_inodes_count: 0,
            used_dirs_count: 0,
            flags: BG_BLOCK_UNINIT,
        };
        assert!(BlockGroup::load_block_bitmap(&disk, 0, 99, &desc).is_err());
    }

    /// A 64-byte descriptor whose block-number high halves are non-zero decodes
    /// to the correct `> 2^32` `Ext4Bid` — the red-line splice. The per-group
    /// counters come from the low half untouched.
    #[ktest]
    fn decode_64byte_descriptor_high_halves() {
        let raw = RawBlockGroup64 {
            lo: RawBlockGroup {
                block_bitmap_lo: 0x1111_2222,
                inode_bitmap_lo: 0x3333_4444,
                inode_table_lo: 0x5555_6666,
                free_blocks_count_lo: 7,
                free_inodes_count_lo: 9,
                used_dirs_count_lo: 3,
                ..Default::default()
            },
            hi: RawBlockGroupHi {
                block_bitmap_hi: 0xA,
                inode_bitmap_hi: 0xB,
                inode_table_hi: 0xC,
                ..Default::default()
            },
        };

        let desc = BlockGroupDesc::from_raw(&raw.lo, Some(&raw.hi));
        assert_eq!(desc.block_bitmap_bid(), 0x0000_000A_1111_2222);
        assert_eq!(desc.inode_bitmap_bid(), 0x0000_000B_3333_4444);
        assert_eq!(desc.inode_table_bid(), 0x0000_000C_5555_6666);
        assert_eq!(desc.free_blocks_count(), 7);
        assert_eq!(desc.free_inodes_count(), 9);
        assert_eq!(desc.used_dirs_count(), 3);

        // The 32-byte (no-hi) path leaves the block numbers at their low halves.
        let desc32 = BlockGroupDesc::from_raw(&raw.lo, None);
        assert_eq!(desc32.block_bitmap_bid(), 0x1111_2222);
        assert_eq!(desc32.inode_bitmap_bid(), 0x3333_4444);
        assert_eq!(desc32.inode_table_bid(), 0x5555_6666);
    }

    /// Two adjacent 64-byte descriptors in a GDT are read at their 64-byte-apart
    /// offsets and each decodes to its own distinct (wide) block numbers — the
    /// desc_size stride plus the wide read.
    #[ktest]
    fn read_desc_64byte_stride() {
        let disk = Ext4MemoryDisk::new(4);
        let gdt_base = BLOCK_SIZE; // GDT at block 1, as the fixture lays it out.
        let desc_size: u16 = 64;

        let g0 = RawBlockGroup64 {
            lo: RawBlockGroup {
                block_bitmap_lo: 0x10,
                inode_bitmap_lo: 0x11,
                inode_table_lo: 0x12,
                ..Default::default()
            },
            hi: RawBlockGroupHi {
                block_bitmap_hi: 1,
                inode_table_hi: 2,
                ..Default::default()
            },
        };
        let g1 = RawBlockGroup64 {
            lo: RawBlockGroup {
                block_bitmap_lo: 0x20,
                inode_bitmap_lo: 0x21,
                inode_table_lo: 0x22,
                ..Default::default()
            },
            hi: RawBlockGroupHi {
                block_bitmap_hi: 3,
                inode_table_hi: 4,
                ..Default::default()
            },
        };

        let off0 = gdt_base;
        let off1 = gdt_base + desc_size as usize;
        disk.segment().write_val(off0, &g0).unwrap();
        disk.segment().write_val(off1, &g1).unwrap();

        let d0 = BlockGroup::read_desc(&disk, off0, desc_size, 0, None).unwrap();
        let d1 = BlockGroup::read_desc(&disk, off1, desc_size, 1, None).unwrap();
        assert_eq!(d0.block_bitmap_bid(), (1u64 << 32) | 0x10);
        assert_eq!(d0.inode_table_bid(), (2u64 << 32) | 0x12);
        assert_eq!(d1.block_bitmap_bid(), (3u64 << 32) | 0x20);
        assert_eq!(d1.inode_bitmap_bid(), 0x21);
        assert_eq!(d1.inode_table_bid(), (4u64 << 32) | 0x22);
    }

    /// A 32-byte descriptor stamped with its crc32c `bg_checksum` verifies; a
    /// corrupted field, a wrong group number, or a wrong seed each fail with
    /// `EUCLEAN`. The checksum depends on the group number, so the same bytes at
    /// a different group index do not verify.
    #[ktest]
    fn group_desc_checksum_round_trip() {
        let seed = FsCsumSeed::new(0x1234_5678);
        let mut lo = RawBlockGroup {
            block_bitmap_lo: 0x10,
            inode_bitmap_lo: 0x11,
            inode_table_lo: 0x12,
            free_blocks_count_lo: 100,
            free_inodes_count_lo: 50,
            used_dirs_count_lo: 3,
            ..Default::default()
        };
        lo.checksum = BlockGroupDesc::group_desc_checksum(&lo, None, 7, seed);
        BlockGroupDesc::verify_group_desc_checksum(&lo, None, 7, seed).unwrap();

        // Wrong group number: the checksum folds it in.
        assert_eq!(
            BlockGroupDesc::verify_group_desc_checksum(&lo, None, 8, seed)
                .unwrap_err()
                .error(),
            Errno::EUCLEAN
        );
        // Wrong seed.
        assert!(
            BlockGroupDesc::verify_group_desc_checksum(&lo, None, 7, FsCsumSeed::new(0x1234_5679))
                .is_err()
        );
        // Corrupted body.
        let mut bad = lo;
        bad.free_blocks_count_lo = 101;
        assert!(BlockGroupDesc::verify_group_desc_checksum(&bad, None, 7, seed).is_err());
    }

    /// The 64-byte descriptor folds its high-half tail (including the bitmap
    /// checksum high halves) into `bg_checksum`, so a change there is caught.
    #[ktest]
    fn group_desc_checksum_covers_high_tail() {
        let seed = FsCsumSeed::new(0xABCD);
        let lo = RawBlockGroup {
            block_bitmap_lo: 0x20,
            ..Default::default()
        };
        let mut hi = RawBlockGroupHi {
            block_bitmap_hi: 1,
            block_bitmap_csum_hi: 0x9999,
            ..Default::default()
        };
        let csum = BlockGroupDesc::group_desc_checksum(&lo, Some(&hi), 2, seed);
        let mut lo_stamped = lo;
        lo_stamped.checksum = csum;
        BlockGroupDesc::verify_group_desc_checksum(&lo_stamped, Some(&hi), 2, seed).unwrap();

        // A change in the high tail changes the checksum.
        hi.block_bitmap_csum_hi = 0x8888;
        assert_ne!(
            BlockGroupDesc::group_desc_checksum(&lo, Some(&hi), 2, seed),
            csum
        );
    }
}
