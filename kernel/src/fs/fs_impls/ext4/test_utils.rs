// SPDX-License-Identifier: MPL-2.0

//! In-memory block device and image fixtures for ext4 kernel-mode tests.
//!
//! These fixtures hand-build a minimal-feature image (extent + filetype, no
//! checksums/64-bit/flex_bg) directly in memory, since `mke2fs` cannot run
//! inside a kernel test. This read-only build only lays down the on-disk state
//! the read paths consume — a superblock, per-group descriptors, a root inode,
//! and (via [`Ext4Fixture::write_data_block`] / [`Ext4Fixture::write_raw_inode`])
//! whatever data blocks and inodes a test wants to read back.

use core::fmt;

use aster_block::{
    BlockDeviceMeta,
    bio::{BioEnqueueError, BioType, SubmittedBio},
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{HasSize, io::util::HasVmReaderWriter};

use super::{
    block_group::RawBlockGroup,
    fs::{Ext4, ROOT_INO},
    inode::{FileFlags, RawInode},
    prelude::*,
    super_block::{MAGIC_NUM, RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
};

/// An in-memory block device backed by a zeroed frame segment.
///
/// Read-only: only read bios are serviced. `build()` seeds the image by writing
/// straight into the backing [`Segment`], never through a write bio, so no write
/// path is needed.
pub(super) struct Ext4MemoryDisk {
    segment: Segment<()>,
}

impl Ext4MemoryDisk {
    pub(super) fn new(nblocks: usize) -> Self {
        let npages = (nblocks * BLOCK_SIZE).div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(npages)
            .unwrap();
        Self { segment }
    }

    pub(super) fn segment(&self) -> &Segment<()> {
        &self.segment
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
        let mut cur_device_ofs = bio.sid_range().start.to_raw() as usize * SECTOR_SIZE;
        for seg in bio.segments() {
            let io_size = match bio.type_() {
                BioType::Read => seg
                    .inner_dma_slice()
                    .writer()
                    .unwrap()
                    .write(self.segment.reader().skip(cur_device_ofs)),
                // Read-only mount: no write or flush bio can reach here (all
                // write paths return `EROFS`), so anything else is unsupported.
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
    #[expect(dead_code)]
    pub sb: SuperBlock,
}

impl Ext4Fixture {
    /// Writes raw bytes into data block `block`.
    pub(super) fn write_data_block(&self, block: u32, data: &[u8]) {
        self.disk
            .segment()
            .write_bytes(block as usize * BLOCK_SIZE, data)
            .unwrap();
    }

    /// Writes a raw inode into the group-0 inode table at inode number `ino`.
    pub(super) fn write_raw_inode(&self, ino: u32, raw: &RawInode) {
        let offset = INODE_TABLE_BID as usize * BLOCK_SIZE + (ino - 1) as usize * INODE_SIZE;
        self.disk.segment().write_val(offset, raw).unwrap();
    }
}

/// Builds a minimal single-purpose ext4 image with a fixed group-0 layout:
/// block 0 = superblock, block 1 = GDT, 2 = block bitmap, 3 = inode bitmap,
/// 4.. = inode table.
///
/// Read-only: the free-block/inode counters are left at zero and the bitmaps are
/// left all-zero. The read paths never consult them (statfs only echoes the
/// superblock counters), so there is no allocator state to seed.
pub(super) struct Ext4FixtureBuilder {
    blocks_per_group: u32,
    inodes_per_group: u32,
    nblocks: usize,
    /// Blocks reserved for privileged processes (`s_r_blocks_count`); `statfs`
    /// subtracts them from `bfree` to report `bavail`.
    reserved_blocks: u32,
    /// Volume UUID (`s_uuid`); `statfs` folds its low 8 bytes into `f_fsid`.
    uuid: [u8; 16],
}

impl Ext4FixtureBuilder {
    pub(super) fn new(blocks_per_group: u32, inodes_per_group: u32, nblocks: usize) -> Self {
        Self {
            blocks_per_group,
            inodes_per_group,
            nblocks,
            reserved_blocks: 0,
            uuid: [0; 16],
        }
    }

    /// Sets `s_r_blocks_count` so `statfs` has reserved blocks to subtract from
    /// `bfree` when reporting `bavail`.
    pub(super) fn with_reserved_blocks(mut self, blocks: u32) -> Self {
        self.reserved_blocks = blocks;
        self
    }

    /// Sets `s_uuid` so `statfs` has a non-zero `f_fsid` (its low 8 bytes).
    pub(super) fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    pub(super) fn build(self) -> Result<Ext4Fixture> {
        let nr_groups = (self.nblocks as u32 - 1) / self.blocks_per_group + 1;
        let inodes_count = nr_groups * self.inodes_per_group;

        let raw_sb = RawSuperBlock {
            inodes_count,
            blocks_count: self.nblocks as u32,
            reserved_blocks_count: self.reserved_blocks,
            free_blocks_count: 0,
            free_inodes_count: 0,
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
            inode_size: INODE_SIZE as u16,
            // Read-only fixture: no journal.
            feature_compat: 0,
            feature_incompat: 0x2 | 0x40, // FILETYPE | EXTENTS
            feature_ro_compat: 0x1,       // SPARSE_SUPER
            uuid: self.uuid,
            journal_ino: 0,
            ..Default::default()
        };
        let sb = SuperBlock::try_from(raw_sb)?;

        let disk = Arc::new(Ext4MemoryDisk::new(self.nblocks));
        disk.segment()
            .write_val(SUPER_BLOCK_OFFSET, &raw_sb)
            .unwrap();

        // Lay out a descriptor per group. Each group `g` keeps its metadata at
        // fixed in-group offsets: block bitmap at +2, inode bitmap at +3, inode
        // table at +4. The bitmaps and free counters are left zeroed — the read
        // paths never consult them.
        for g in 0..nr_groups {
            let group_first = g * self.blocks_per_group; // first_data_block == 0
            let raw_gd = RawBlockGroup {
                block_bitmap_lo: group_first + 2,
                inode_bitmap_lo: group_first + 3,
                inode_table_lo: group_first + INODE_TABLE_BID,
                free_blocks_count_lo: 0,
                free_inodes_count_lo: 0,
                ..Default::default()
            };
            disk.segment()
                .write_val(
                    BLOCK_SIZE + g as usize * size_of::<RawBlockGroup>(),
                    &raw_gd,
                )
                .unwrap();
        }

        let root = make_root_dir_inode();
        let root_offset =
            INODE_TABLE_BID as usize * BLOCK_SIZE + (ROOT_INO - 1) as usize * INODE_SIZE;
        disk.segment().write_val(root_offset, &root).unwrap();

        let ext4 = Ext4::open(disk.clone() as Arc<dyn BlockDevice>)?;
        Ok(Ext4Fixture { disk, ext4, sb })
    }
}

/// A root-directory inode: link count 2, `EXT4_EXTENTS_FL` set, and a valid
/// **empty** extent root.
fn make_root_dir_inode() -> RawInode {
    let mut raw = RawInode {
        mode: 0o040755,
        size_lo: BLOCK_SIZE as u32,
        link_count: 2,
        sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
        flags: FileFlags::EXTENTS.bits(),
        extra_isize: 32,
        ..Default::default()
    };
    // A valid **empty** extent root — the root must parse, since
    // `ExtentTree::try_new` validates it when the inode is loaded. Tests that
    // read actual root-directory data overwrite ino 2 with a populated inode.
    raw.block[0] = 0xF30A; // eh_magic | eh_entries(=0)
    raw.block[1] = 4; // eh_max=4, eh_depth=0
    raw
}

/// Builds a regular-file inode mapping logical block 0 to `data_block` via a
/// depth-0 inline extent.
pub(super) fn make_file_inode(data_block: u32, size: u32) -> RawInode {
    let mut raw = RawInode {
        mode: 0o100644, // S_IFREG | 0644
        size_lo: size,
        link_count: 1,
        sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
        flags: FileFlags::EXTENTS.bits(),
        extra_isize: 32,
        ..Default::default()
    };
    // Inline extent root in `i_block`: a 12-byte header (magic 0xF30A, 1 entry,
    // max 4, depth 0) followed by one extent mapping logical block 0 to
    // `data_block` with length 1. Each `i_block` word packs two 16-bit fields.
    raw.block[0] = 0xF30A | (1 << 16); // eh_magic | eh_entries
    raw.block[1] = 4; // eh_max=4, eh_depth=0
    raw.block[2] = 0; // eh_generation
    raw.block[3] = 0; // ee_block = 0
    raw.block[4] = 1; // ee_len=1, ee_start_hi=0
    raw.block[5] = data_block; // ee_start_lo
    raw
}

/// Builds an empty regular-file inode: size 0, `i_blocks` 0, and an empty
/// inline extent root (header with 0 entries).
pub(super) fn make_empty_file_inode() -> RawInode {
    let mut raw = RawInode {
        mode: 0o100644, // S_IFREG | 0644
        size_lo: 0,
        link_count: 1,
        sector_count: 0,
        flags: FileFlags::EXTENTS.bits(),
        extra_isize: 32,
        ..Default::default()
    };
    // Empty extent root: 12-byte header (magic 0xF30A, 0 entries, max 4, depth
    // 0), no extents. Each `i_block` word packs two 16-bit fields.
    raw.block[0] = 0xF30A; // eh_magic | eh_entries(=0)
    raw.block[1] = 4; // eh_max=4, eh_depth=0
    raw.block[2] = 0; // eh_generation
    raw
}

/// Builds a regular-file inode whose `i_block` holds a single *unwritten*
/// (preallocated) inline extent mapping logical blocks `[0, len)` to physical
/// `[data_block, data_block + len)`. The blocks count toward `i_blocks`, but the
/// extent is flagged unwritten, so the read path serves them as zeros. `size` is
/// the logical file size in bytes.
pub(super) fn make_unwritten_file_inode(data_block: u32, len: u16, size: u32) -> RawInode {
    let mut raw = RawInode {
        mode: 0o100644, // S_IFREG | 0644
        size_lo: size,
        link_count: 1,
        sector_count: (len as u32) * (BLOCK_SIZE / SECTOR_SIZE) as u32,
        flags: FileFlags::EXTENTS.bits(),
        extra_isize: 32,
        ..Default::default()
    };
    // Inline extent root: header (magic, 1 entry, max 4, depth 0) + one extent.
    // An unwritten extent encodes its length biased by 32768 (`MAX_WRITTEN_LEN`).
    raw.block[0] = 0xF30A | (1 << 16); // eh_magic | eh_entries
    raw.block[1] = 4; // eh_max=4, eh_depth=0
    raw.block[2] = 0; // eh_generation
    raw.block[3] = 0; // ee_block = 0
    // ee_len (low 16, biased for unwritten) | ee_start_hi (high 16, = 0).
    raw.block[4] = (len + 32768) as u32;
    raw.block[5] = data_block; // ee_start_lo
    raw
}

/// Builds a directory inode (type Dir, one block) mapping logical block 0 to
/// `data_block`.
pub(super) fn make_dir_inode(data_block: u32) -> RawInode {
    let mut raw = make_file_inode(data_block, BLOCK_SIZE as u32);
    raw.mode = 0o040755; // S_IFDIR | 0755
    raw.link_count = 2;
    raw
}

/// Builds one directory data block holding `entries` of `(ino, name,
/// file_type)`. The last entry's `rec_len` is extended to fill the block.
pub(super) fn make_dir_block(entries: &[(u32, &str, u8)]) -> Vec<u8> {
    let mut block = vec![0u8; BLOCK_SIZE];
    let mut offset = 0;
    for (i, (ino, name, file_type)) in entries.iter().enumerate() {
        let name_bytes = name.as_bytes();
        let rec_len = if i + 1 == entries.len() {
            BLOCK_SIZE - offset
        } else {
            (name_bytes.len() + 8).next_multiple_of(4)
        };
        block[offset..offset + 4].copy_from_slice(&ino.to_le_bytes());
        block[offset + 4..offset + 6].copy_from_slice(&(rec_len as u16).to_le_bytes());
        block[offset + 6] = name_bytes.len() as u8;
        block[offset + 7] = *file_type;
        block[offset + 8..offset + 8 + name_bytes.len()].copy_from_slice(name_bytes);
        offset += rec_len;
    }
    block
}
