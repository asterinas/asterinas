// SPDX-License-Identifier: MPL-2.0

//! In-memory block device and image fixtures for ext4 kernel-mode tests.
//!
//! Fixtures build a minimal-feature image directly in memory because `mke2fs`
//! cannot run inside a kernel test.

use core::{
    fmt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use aster_block::{
    BlockDeviceMeta,
    bio::{BioEnqueueError, BioType, SubmittedBio},
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{HasSize, io::util::HasVmReaderWriter};

use super::{
    block_group::RawBlockGroup,
    fs::{Ext4, MountFlavor, ROOT_INO},
    inode::{EXTENTS_FL, RawInode, empty_extent_root},
    prelude::*,
    super_block::{MAGIC_NUM, RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
};

/// An in-memory block device backed by a zeroed frame segment.
pub(super) struct Ext4MemoryDisk {
    segment: Segment<()>,
    flush_count: AtomicUsize,
    /// When set, every write bio fails with `IoError`. Lets tests force a
    /// metadata writeback failure (e.g. to exercise `create_inode` rollback).
    fail_writes: AtomicBool,
}

impl Ext4MemoryDisk {
    pub(super) fn new(nblocks: usize) -> Self {
        let npages = (nblocks * BLOCK_SIZE).div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(npages)
            .unwrap();
        Self {
            segment,
            flush_count: AtomicUsize::new(0),
            fail_writes: AtomicBool::new(false),
        }
    }

    pub(super) fn segment(&self) -> &Segment<()> {
        &self.segment
    }

    /// Makes every subsequent write bio fail (or stops failing them).
    pub(super) fn set_fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::Relaxed);
    }
}

impl Debug for Ext4MemoryDisk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ext4MemoryDisk")
            .field("bytes", &self.segment.size())
            .finish()
    }
}

impl BlockDevice for Ext4MemoryDisk {
    fn enqueue(&self, bio: SubmittedBio) -> core::result::Result<(), BioEnqueueError> {
        if bio.type_() == BioType::Flush {
            self.flush_count.fetch_add(1, Ordering::Relaxed);
            bio.complete(BioStatus::Complete);
            return Ok(());
        }

        if bio.type_() == BioType::Write && self.fail_writes.load(Ordering::Relaxed) {
            bio.complete(BioStatus::IoError);
            return Ok(());
        }

        let mut cur_device_ofs = bio.sid_range().start.to_raw() as usize * SECTOR_SIZE;
        for seg in bio.segments() {
            let io_size = match bio.type_() {
                BioType::Read => seg
                    .inner_dma_slice()
                    .writer()
                    .unwrap()
                    .write(self.segment.reader().skip(cur_device_ofs)),
                BioType::Write => self
                    .segment
                    .writer()
                    .skip(cur_device_ofs)
                    .write(&mut seg.inner_dma_slice().reader().unwrap()),
                _ => {
                    bio.complete(BioStatus::NotSupported);
                    return Ok(());
                }
            };
            cur_device_ofs += io_size;
        }

        bio.complete(BioStatus::Complete);
        Ok(())
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: usize::MAX,
            nr_sectors: self.segment.size() / SECTOR_SIZE,
        }
    }

    fn name(&self) -> &str {
        "ext4-memory-disk"
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(1), MinorId::new(0))
    }
}

/// Fixed group-0 layout used by the fixture builder.
const INODE_SIZE: usize = 256;
const INODE_TABLE_BID: u32 = 4;

/// A mounted ext4 filesystem over an in-memory disk, for tests.
pub(super) struct Ext4Fixture {
    pub disk: Arc<Ext4MemoryDisk>,
    pub ext4: Arc<Ext4>,
    /// The volume's on-disk inode slot size (128 or 256 in fixtures).
    pub inode_size: usize,
}

impl Ext4Fixture {
    /// Writes raw bytes into data block `block`.
    pub(super) fn write_data_block(&self, block: u32, data: &[u8]) {
        self.disk
            .segment()
            .write_bytes(block as usize * BLOCK_SIZE, data)
            .unwrap();
    }

    /// Writes a raw inode into the group-0 inode table at inode number `ino`,
    /// truncated to the volume's inode slot size.
    pub(super) fn write_raw_inode(&self, ino: u32, raw: &RawInode) {
        let offset = INODE_TABLE_BID as usize * BLOCK_SIZE + (ino - 1) as usize * self.inode_size;
        self.disk
            .segment()
            .write_bytes(
                offset,
                &raw.as_bytes()[..self.inode_size.min(size_of::<RawInode>())],
            )
            .unwrap();
    }

    /// Reads back the raw inode at inode number `ino` from the group-0 table,
    /// zero-filling the tail when the volume's slots are shorter than 256
    /// bytes.
    pub(super) fn read_raw_inode(&self, ino: u32) -> RawInode {
        let offset = INODE_TABLE_BID as usize * BLOCK_SIZE + (ino - 1) as usize * self.inode_size;
        let mut bytes = [0u8; size_of::<RawInode>()];
        let n = self.inode_size.min(size_of::<RawInode>());
        self.disk
            .segment()
            .read_bytes(offset, &mut bytes[..n])
            .unwrap();
        RawInode::from_bytes(&bytes)
    }
}

/// Builds a minimal single-purpose ext4 image with a fixed group-0 layout:
/// block 0 = superblock, block 1 = GDT, 2 = block bitmap, 3 = inode bitmap,
/// 4.. = inode table.
pub(super) struct Ext4FixtureBuilder {
    blocks_per_group: u32,
    inodes_per_group: u32,
    nblocks: usize,
    /// When set, mark the group-0 system/reserved blocks as allocated in the
    /// block bitmap and seed the matching free-block counters. Off by default so
    /// the read-only fixtures keep their all-zero bitmap.
    mark_metadata: bool,
    /// When set, override the free-block counters to zero (for ENOSPC tests).
    no_free_blocks: bool,
    /// When set, cap the (single-group) fixture to exactly this many free blocks
    /// by marking all but the top `n` data blocks allocated. For
    /// ENOSPC-mid-operation tests.
    free_block_cap: Option<u32>,
    /// When set, mark the reserved inodes (1..`first_ino`) of group 0 as
    /// allocated in the inode bitmap and seed the matching free-inode counters.
    /// Off by default so the read-only fixtures keep their all-zero inode
    /// bitmap and zero counters.
    mark_inode_metadata: bool,
    /// When set, override the free-inode counters to zero (for inode-ENOSPC
    /// tests). Implies the inode metadata is marked.
    no_free_inodes: bool,
    /// When set, additionally mark this group-0 inode allocated in the inode
    /// bitmap and decrement the free-inode counters by one. Used to reserve a
    /// pre-placed test directory inode so the allocator does not hand its number
    /// back out to a freshly created child. Requires `mark_inode_metadata`.
    reserved_inode: Option<u32>,
    /// On-disk inode slot size; defaults to 256.
    inode_size: usize,
    /// Omit the EXTENTS incompat feature (an ext2-format volume).
    without_extents: bool,
    /// Omit the FILETYPE incompat feature (used to assert mount rejection).
    /// Extra `ro_compat` feature bits OR'd into the superblock (used to
    /// assert the unknown-ro-compat mount rejection).
    extra_ro_compat: u32,
    /// Extra `compat` feature bits OR'd into the superblock (used to assert
    /// the ext2-flavor journal rejection).
    extra_compat: u32,
    /// The type name to mount the fixture volume under; `Ext4` by default.
    flavor: MountFlavor,
}

impl Ext4FixtureBuilder {
    pub(super) fn new(blocks_per_group: u32, inodes_per_group: u32, nblocks: usize) -> Self {
        Self {
            blocks_per_group,
            inodes_per_group,
            nblocks,
            mark_metadata: false,
            no_free_blocks: false,
            free_block_cap: None,
            mark_inode_metadata: false,
            no_free_inodes: false,
            reserved_inode: None,
            inode_size: INODE_SIZE,
            // No extent volumes until the extent engine lands.
            without_extents: true,
            extra_ro_compat: 0,
            extra_compat: 0,
            flavor: MountFlavor::Ext4,
        }
    }

    /// Reserves an extra group-0 inode (beyond the reserved 1..`first_ino`): its
    /// bitmap bit is marked allocated and the free-inode counters are reduced by
    /// one, so the allocator skips it. Used by directory fixtures whose
    /// pre-placed directory inode would otherwise be re-handed-out as a child.
    /// Sets the volume's on-disk inode slot size (e.g. 128 for old-style
    /// volumes). Must be a power of two in `128..=BLOCK_SIZE`.
    pub(super) fn inode_size(mut self, inode_size: usize) -> Self {
        self.inode_size = inode_size;
        self
    }

    /// Builds an ext2-format volume: the EXTENTS incompat feature is absent,
    /// so every inode maps through the indirect engine.
    pub(super) fn without_extents_feature(mut self) -> Self {
        self.without_extents = true;
        self
    }

    pub(super) fn with_reserved_inode(mut self, ino: u32) -> Self {
        self.mark_inode_metadata = true;
        self.reserved_inode = Some(ino);
        self
    }

    /// Marks the group-0 metadata + reserved blocks as allocated in the block
    /// bitmap and seeds free-block counters accordingly, giving allocator tests
    /// a realistic starting image.
    pub(super) fn with_block_bitmap_metadata_marked(mut self) -> Self {
        self.mark_metadata = true;
        self
    }

    /// Caps the single-group fixture to exactly `n` free blocks (marking all but
    /// the top `n` data blocks allocated). For tests that must run the allocator
    /// out of space partway through a multi-block operation.
    pub(super) fn with_free_blocks(mut self, n: u32) -> Self {
        self.free_block_cap = Some(n);
        self.mark_metadata = true;
        self
    }

    /// Marks the reserved inodes (1..`first_ino`) of group 0 as allocated in the
    /// inode bitmap and seeds the free-inode counters accordingly, giving inode
    /// allocator tests a realistic starting image.
    pub(super) fn with_inode_bitmap_metadata_marked(mut self) -> Self {
        self.mark_inode_metadata = true;
        self
    }

    pub(super) fn build(self) -> Result<Ext4Fixture> {
        let nr_groups = (self.nblocks as u32 - 1) / self.blocks_per_group + 1;
        let inodes_count = nr_groups * self.inodes_per_group;
        let inode_table_blocks = self.inodes_per_group / (BLOCK_SIZE / self.inode_size) as u32;

        // Each group's system zone spans, from its first block: the superblock
        // region (block 0 in group 0), the GDT block, block bitmap, inode bitmap,
        // and the inode-table blocks. first_data_block is 0 in this fixture.
        let metadata_end_block = INODE_TABLE_BID + inode_table_blocks; // exclusive

        // Sum the free blocks across all groups so the superblock counter matches
        // the per-group descriptors.
        let total_free: u32 = if let Some(n) = self.free_block_cap {
            n
        } else if self.no_free_blocks || !self.mark_metadata {
            0
        } else {
            (0..nr_groups)
                .map(|g| {
                    let group_first = g * self.blocks_per_group;
                    let group_size = if g == nr_groups - 1 {
                        self.nblocks as u32 - group_first
                    } else {
                        self.blocks_per_group
                    };
                    group_size - metadata_end_block
                })
                .sum()
        };

        // Reserved inodes (1..first_ino) occupy the low bits of group 0's inode
        // bitmap; `first_ino` is 11 in this fixture, so bits 0..10 are reserved.
        const FIRST_INO: u32 = 11;
        let reserved_inodes = FIRST_INO - 1;

        // Per-group free-inode counts, summed for the superblock counter. Only
        // group 0 carries the reserved inodes; the zero override applies to the
        // single-group fixtures used by the inode-ENOSPC test.
        // An extra reserved inode (a pre-placed test directory) lives in group 0
        // and removes one free inode from group 0's count.
        let extra_reserved = self.reserved_inode.is_some() as u32;
        let total_free_inodes: u32 = if self.no_free_inodes || !self.mark_inode_metadata {
            0
        } else {
            (0..nr_groups)
                .map(|g| {
                    if g == 0 {
                        self.inodes_per_group - reserved_inodes - extra_reserved
                    } else {
                        self.inodes_per_group
                    }
                })
                .sum()
        };

        let raw_sb = RawSuperBlock {
            inodes_count,
            blocks_count: self.nblocks as u32,
            free_blocks_count: total_free,
            free_inodes_count: total_free_inodes,
            first_data_block: 0,
            log_block_size: 2,
            log_frag_size: 2,
            blocks_per_group: self.blocks_per_group,
            frags_per_group: self.blocks_per_group,
            inodes_per_group: self.inodes_per_group,
            magic: MAGIC_NUM,
            state: 1,      // VALID
            errors: 1,     // Continue
            creator_os: 0, // Linux
            rev_level: 1,  // Dynamic
            first_ino: 11,
            inode_size: self.inode_size as u16,
            feature_compat: self.extra_compat,
            feature_incompat: 0x2 | if self.without_extents { 0 } else { 0x40 }, // FILETYPE | EXTENTS
            feature_ro_compat: 0x1 | self.extra_ro_compat, // SPARSE_SUPER + extras
            ..Default::default()
        };
        // Validate the raw superblock up front; the fixture no longer keeps the
        // parsed `SuperBlock` (tests read it back via `ext4.super_block()`).
        SuperBlock::try_from(raw_sb)?;

        let disk = Arc::new(Ext4MemoryDisk::new(self.nblocks));
        disk.segment()
            .write_val(SUPER_BLOCK_OFFSET, &raw_sb)
            .unwrap();

        // Lay out a descriptor (and, when marking, a block bitmap) per group.
        // Each group `g` keeps its metadata at fixed in-group offsets: block
        // bitmap at +2, inode bitmap at +3, inode table at +4.
        for g in 0..nr_groups {
            let group_first = g * self.blocks_per_group; // first_data_block == 0
            let group_size = if g == nr_groups - 1 {
                self.nblocks as u32 - group_first
            } else {
                self.blocks_per_group
            };
            // `mark_end` is the exclusive bit up to which the bitmap is marked
            // allocated; capping leaves only the top `n` blocks free.
            let (free, mark_end) = if let Some(n) = self.free_block_cap {
                (n, group_size - n)
            } else if self.no_free_blocks {
                (0, metadata_end_block)
            } else if self.mark_metadata {
                (group_size - metadata_end_block, metadata_end_block)
            } else {
                (0, metadata_end_block)
            };

            // Per-group inode bookkeeping. `inode_mark_end` is the exclusive bit
            // up to which group `g`'s inode bitmap is marked allocated.
            let reserved_in_group = if g == 0 { reserved_inodes } else { 0 };
            let extra_reserved_in_group = if g == 0 { extra_reserved } else { 0 };
            let (free_inodes, inode_mark_end) = if self.no_free_inodes {
                (0, self.inodes_per_group)
            } else if self.mark_inode_metadata {
                (
                    self.inodes_per_group - reserved_in_group - extra_reserved_in_group,
                    reserved_in_group,
                )
            } else {
                (0, 0)
            };

            let raw_gd = RawBlockGroup {
                block_bitmap_lo: group_first + 2,
                inode_bitmap_lo: group_first + 3,
                inode_table_lo: group_first + INODE_TABLE_BID,
                free_blocks_count_lo: free as u16,
                free_inodes_count_lo: free_inodes as u16,
                ..Default::default()
            };
            disk.segment()
                .write_val(
                    BLOCK_SIZE + g as usize * size_of::<RawBlockGroup>(),
                    &raw_gd,
                )
                .unwrap();

            if self.mark_metadata {
                // Mark the in-group allocated zone (bits 0..mark_end) — the system
                // zone, plus extra blocks when capping free space. LSB-first.
                let mut bitmap = vec![0u8; BLOCK_SIZE];
                for bit in 0..mark_end as usize {
                    bitmap[bit / 8] |= 1 << (bit % 8);
                }
                disk.segment()
                    .write_bytes((group_first as usize + 2) * BLOCK_SIZE, &bitmap)
                    .unwrap();
            }

            if self.mark_inode_metadata {
                // Mark the in-group reserved/capped inode bits (0..inode_mark_end)
                // allocated in this group's inode bitmap. LSB-first.
                let mut inode_bitmap = vec![0u8; BLOCK_SIZE];
                for bit in 0..inode_mark_end as usize {
                    inode_bitmap[bit / 8] |= 1 << (bit % 8);
                }
                // Mark the extra reserved inode (a pre-placed test directory) so
                // the allocator skips its number.
                if g == 0
                    && let Some(ino) = self.reserved_inode
                {
                    let bit = (ino - 1) as usize;
                    inode_bitmap[bit / 8] |= 1 << (bit % 8);
                }
                disk.segment()
                    .write_bytes((group_first as usize + 3) * BLOCK_SIZE, &inode_bitmap)
                    .unwrap();
            }
        }

        let root = make_root_dir_inode(!self.without_extents);
        let root_offset =
            INODE_TABLE_BID as usize * BLOCK_SIZE + (ROOT_INO - 1) as usize * self.inode_size;
        disk.segment()
            .write_bytes(
                root_offset,
                &root.as_bytes()[..self.inode_size.min(size_of::<RawInode>())],
            )
            .unwrap();

        let ext4 = Ext4::open(disk.clone() as Arc<dyn BlockDevice>, self.flavor, None)?;
        Ok(Ext4Fixture {
            disk,
            ext4,
            inode_size: self.inode_size,
        })
    }
}

/// A root-directory inode in the volume's format (left empty here; the
/// directory data block is populated when a later task needs to read it),
/// link count 2. Extent volumes get an empty extent root with
/// `EXT4_EXTENTS_FL` set; ext2-format volumes get a zeroed pointer array.
fn make_root_dir_inode(extent_based: bool) -> RawInode {
    let mut raw = RawInode {
        mode: 0o040755,
        size_lo: BLOCK_SIZE as u32,
        link_count: 2,
        sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
        extra_isize: 32,
        ..Default::default()
    };
    if extent_based {
        raw.flags = EXTENTS_FL;
        // A valid, empty extent root: the flag alone with a zeroed `i_block`
        // would fail extent-header validation on the first root-inode read.
        raw.block = empty_extent_root();
    }
    raw
}

/// Builds an ext2-style regular-file inode: no `EXTENTS` flag, one direct
/// block pointer to `data_block`, so loading it dispatches to the indirect
/// engine.
pub(super) fn make_indirect_file_inode(data_block: u32, size: u32) -> RawInode {
    let mut raw = RawInode {
        mode: 0o100644, // S_IFREG | 0644
        size_lo: size,
        link_count: 1,
        sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
        flags: 0,
        extra_isize: 32,
        ..Default::default()
    };
    raw.block[0] = data_block;
    raw
}

/// Builds an empty regular-file inode: size 0, `i_blocks` 0, and a zeroed
/// pointer array. Suitable as the target of a fresh buffered write whose
/// allocation comes from the write path.
pub(super) fn make_empty_file_inode() -> RawInode {
    RawInode {
        mode: 0o100644, // S_IFREG | 0644
        size_lo: 0,
        link_count: 1,
        sector_count: 0,
        extra_isize: 32,
        ..Default::default()
    }
}

/// Writes a single 32-bit block pointer into slot `index` of the on-disk
/// block `bid`, for building ext2-style indirect-block chains in tests.
pub(super) fn write_indirect_ptr(disk: &Ext4MemoryDisk, bid: u32, index: u32, next: u32) {
    let offset = Bid::new(bid as u64).to_offset() + (index as usize) * size_of::<u32>();
    disk.segment().write_val(offset, &next).unwrap();
}
