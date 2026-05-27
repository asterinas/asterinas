// SPDX-License-Identifier: MPL-2.0

//! Test-only fixtures and helpers for ext2 kernel tests.

use core::{
    fmt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use aster_block::{
    BLOCK_SIZE, BlockDevice, BlockDeviceMeta, SECTOR_SIZE,
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    id::Bid,
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{FrameAllocOptions, HasSize, Segment, VmIo, io::util::HasVmReaderWriter};

use super::{
    block_group::RawBlockGroup,
    fs::{Ext2, ROOT_INO},
    inode::{FilePerm, Inode, RAW_BLOCK_PTRS_LEN, RawInode},
    super_block::{
        ErrorsBehavior, FsState, MAGIC_NUM, OsId, RawSuperBlock, RevLevel, SUPER_BLOCK_OFFSET,
    },
};
use crate::{
    fs::{file::InodeType, fs_impls::ext2::super_block::SuperBlock},
    prelude::{return_errno_with_message, *},
    time::clocks,
};
const DOT_BYTE: &[u8] = b".";
const DOT_DOT_BYTE: &[u8] = b"..";

// ===========================================================================
// Layer 0: Primitives — assertions, bit manipulation
// ===========================================================================

macro_rules! assert_errno {
    ($expr:expr, $errno:expr) => {
        match $expr {
            Err(e) => assert_eq!(e.error(), $errno),
            Ok(_) => panic!("expected Err({:?}), got Ok(_)", $errno),
        }
    };
    ($expr:expr, $errno:expr, $($msg:tt)+) => {
        match $expr {
            Err(e) => assert_eq!(e.error(), $errno, $($msg)+),
            Ok(_) => panic!("expected Err({:?}), got Ok(_): {}", $errno, format_args!($($msg)+)),
        }
    };
}
pub(super) use assert_errno;

pub(super) fn set_bit_lsb0(buf: &mut [u8], bit: usize) {
    let byte = bit / 8;
    let bit_in_byte = bit % 8;
    buf[byte] |= 1u8 << bit_in_byte;
}

// ===========================================================================
// Layer 1: Mock devices
// ===========================================================================

pub(super) struct Ext2MemoryDisk {
    segment: Segment<()>,
    flush_count: AtomicUsize,
    fail_flush: AtomicBool,
}

impl Ext2MemoryDisk {
    pub(super) fn new(nblocks: usize) -> Self {
        let npages = (nblocks * BLOCK_SIZE).div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(npages)
            .unwrap();
        Self {
            segment,
            flush_count: AtomicUsize::new(0),
            fail_flush: AtomicBool::new(false),
        }
    }

    pub(super) fn segment(&self) -> &Segment<()> {
        &self.segment
    }

    pub(super) fn write_super_block(&self, raw: &RawSuperBlock) {
        self.segment.write_val(SUPER_BLOCK_OFFSET, raw).unwrap();
    }

    pub(super) fn write_group_desc_table(&self, sb: &SuperBlock, descs: &[RawBlockGroup]) {
        let table_offset = Bid::new(sb.group_descriptors_bid(0) as u64).to_offset();
        for (idx, desc) in descs.iter().enumerate() {
            let offset = table_offset + idx * size_of::<RawBlockGroup>();
            self.segment.write_val(offset, desc).unwrap();
        }
    }
}

impl Debug for Ext2MemoryDisk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ext2MemoryDisk")
            .field("bytes", &self.segment.size())
            .finish()
    }
}

impl BlockDevice for Ext2MemoryDisk {
    fn enqueue(&self, bio: SubmittedBio) -> core::result::Result<(), BioEnqueueError> {
        if bio.type_() == BioType::Flush {
            self.flush_count.fetch_add(1, Ordering::Relaxed);
            let status = if self.fail_flush.load(Ordering::Relaxed) {
                BioStatus::IoError
            } else {
                BioStatus::Complete
            };
            bio.complete(status);
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
        "ext2-memory-disk"
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(1), MinorId::new(0))
    }
}

// ===========================================================================
// Layer 2: Raw data factories
// ===========================================================================

pub(super) fn make_valid_raw_super_block(nr_block_groups: u32) -> RawSuperBlock {
    let mut raw = RawSuperBlock::default();
    raw.magic = MAGIC_NUM;
    raw.log_block_size = 2;
    raw.log_frag_size = 2;
    raw.state = FsState::VALID.bits();
    raw.errors = ErrorsBehavior::Continue as u16;
    raw.creator_os = OsId::Linux as u32;
    raw.rev_level = RevLevel::GoodOld as u32;
    raw.first_data_block = 0;
    raw.blocks_per_group = 128;
    raw.frags_per_group = raw.blocks_per_group;
    raw.inodes_per_group = 1024;
    raw.inodes_count = nr_block_groups * raw.inodes_per_group;

    let tail_blocks = 64;
    raw.blocks_count = raw.first_data_block
        + 1
        + (nr_block_groups.saturating_sub(1)) * raw.blocks_per_group
        + tail_blocks;
    raw
}

pub(super) fn make_valid_super_block(nr_block_groups: u32) -> SuperBlock {
    SuperBlock::try_from(make_valid_raw_super_block(nr_block_groups)).unwrap()
}

pub(super) fn make_valid_group_desc(sb: &SuperBlock, group_idx: usize) -> RawBlockGroup {
    let first = sb.group_first_block_no(group_idx);
    RawBlockGroup {
        block_bitmap_bid: first,
        inode_bitmap_bid: first + 1,
        inode_table_bid: first + 2,
        free_blocks_count: 0,
        free_inodes_count: 0,
        used_dirs_count: 0,
        pad: 0,
        reserved: [0; 3],
    }
}

pub(super) struct RawInodeBuilder {
    mode: u16,
    link_count: u16,
    dtime: u32,
    size_lo: u32,
    sector_count: u32,
    flags: u32,
    block: [u32; RAW_BLOCK_PTRS_LEN],
}

impl RawInodeBuilder {
    pub(super) fn new(mode: u16) -> Self {
        Self {
            mode,
            link_count: 1,
            dtime: 0,
            size_lo: 0,
            sector_count: 0,
            flags: 0,
            block: [0; RAW_BLOCK_PTRS_LEN],
        }
    }

    pub(super) fn link_count(mut self, v: u16) -> Self {
        self.link_count = v;
        self
    }

    pub(super) fn dtime(mut self, v: u32) -> Self {
        self.dtime = v;
        self
    }

    pub(super) fn size_lo(mut self, v: u32) -> Self {
        self.size_lo = v;
        self
    }

    pub(super) fn sector_count(mut self, v: u32) -> Self {
        self.sector_count = v;
        self
    }

    pub(super) fn block_ptrs(mut self, v: [u32; RAW_BLOCK_PTRS_LEN]) -> Self {
        self.block = v;
        self
    }

    pub(super) fn build(self) -> RawInode {
        RawInode {
            mode: self.mode,
            uid: 0,
            size_lo: self.size_lo,
            atime: 0,
            ctime: 0,
            mtime: 0,
            dtime: self.dtime,
            gid: 0,
            link_count: self.link_count,
            sector_count: self.sector_count,
            flags: self.flags,
            osd1: 0,
            block: self.block,
            generation: 0,
            file_acl: 0,
            size_high: 0,
            faddr: 0,
            frag: 0,
            fsize: 0,
            pad1: 0,
            uid_high: 0,
            gid_high: 0,
            reserved2: 0,
        }
    }
}

// ===========================================================================
// Layer 3: Disk layout writers
// ===========================================================================

pub(super) fn write_indirect_ptr(disk: &Ext2MemoryDisk, bid: u32, index: u32, next: u32) {
    let offset = Bid::new(bid as u64).to_offset() + (index as usize) * size_of::<u32>();
    disk.segment().write_val(offset, &next).unwrap();
}

pub(super) fn write_raw_inode_to_disk(
    sb: &SuperBlock,
    descs: &[RawBlockGroup],
    ino: u32,
    raw: &RawInode,
    disk: &Ext2MemoryDisk,
) {
    let nr_inodes_per_group = sb.nr_inodes_per_group();
    let group_idx = ((ino - 1) / nr_inodes_per_group) as usize;
    let inode_idx = (ino - 1) % nr_inodes_per_group;

    let inode_size = sb.inode_size();
    let offset_bytes = (inode_idx as usize).saturating_mul(inode_size);
    let block_index = offset_bytes / BLOCK_SIZE;
    let offset_in_block = offset_bytes % BLOCK_SIZE;

    let table_block = descs[group_idx].inode_table_bid + block_index as u32;
    let table_bid = Bid::new(table_block as u64);
    disk.segment()
        .write_val(table_bid.to_offset() + offset_in_block, raw)
        .unwrap();
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Group0Layout {
    pub group_desc_bid: u32,
    pub block_bitmap_bid: u32,
    pub inode_bitmap_bid: u32,
    pub inode_table_bid: u32,
    pub first_data_bid: u32,
}

pub(super) fn group0_layout(sb: &SuperBlock) -> Group0Layout {
    let sb_bid = sb.bid(0);
    let group_desc_bid = sb.group_descriptors_bid(0);

    let skip_reserved = |mut b: u32| -> u32 {
        while b == sb_bid || b == group_desc_bid {
            b += 1;
        }
        b
    };

    let mut next = skip_reserved(sb.group_first_block_no(0));
    let block_bitmap_bid = next;
    next = skip_reserved(next + 1);
    let inode_bitmap_bid = next;
    next = skip_reserved(next + 1);
    let inode_table_bid = next;
    let first_data_bid = inode_table_bid + sb.nr_inode_table_blocks_per_group();

    Group0Layout {
        group_desc_bid,
        block_bitmap_bid,
        inode_bitmap_bid,
        inode_table_bid,
        first_data_bid,
    }
}

pub(super) fn validate_group0_layout(sb: &SuperBlock, layout: &Group0Layout) -> Result<()> {
    let first = sb.group_first_block_no(0);
    let last = sb.group_last_block_no(0);

    let in_range_fn = |block: u32| block >= first && block <= last;
    if !in_range_fn(layout.group_desc_bid)
        || !in_range_fn(layout.block_bitmap_bid)
        || !in_range_fn(layout.inode_bitmap_bid)
        || !in_range_fn(layout.inode_table_bid)
    {
        return_errno_with_message!(Errno::EINVAL, "test layout block out of group range");
    }

    let inode_table_last = layout
        .inode_table_bid
        .saturating_add(sb.nr_inode_table_blocks_per_group())
        .saturating_sub(1);
    if inode_table_last > last {
        return_errno_with_message!(Errno::EINVAL, "test layout inode table out of range");
    }

    if layout.block_bitmap_bid == layout.inode_bitmap_bid
        || layout.block_bitmap_bid == layout.group_desc_bid
        || layout.inode_bitmap_bid == layout.group_desc_bid
    {
        return_errno_with_message!(Errno::EINVAL, "test layout metadata blocks overlap");
    }

    if layout.block_bitmap_bid >= layout.inode_table_bid
        && layout.block_bitmap_bid <= inode_table_last
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "test layout block bitmap overlaps inode table"
        );
    }
    if layout.inode_bitmap_bid >= layout.inode_table_bid
        && layout.inode_bitmap_bid <= inode_table_last
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "test layout inode bitmap overlaps inode table"
        );
    }
    if layout.group_desc_bid >= layout.inode_table_bid && layout.group_desc_bid <= inode_table_last
    {
        return_errno_with_message!(Errno::EINVAL, "test layout group desc overlaps inode table");
    }

    if layout.first_data_bid <= first {
        return_errno_with_message!(Errno::EINVAL, "test layout first_data invalid");
    }

    Ok(())
}

fn make_root_raw_inode(root_bid: u32) -> RawInode {
    RawInodeBuilder::new(InodeType::Dir as u16 | 0o755)
        .link_count(2)
        .size_lo(BLOCK_SIZE as u32)
        .sector_count((BLOCK_SIZE / SECTOR_SIZE) as u32)
        .block_ptrs({
            let mut ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
            ptrs[0] = root_bid;
            ptrs
        })
        .build()
}

pub(super) fn write_simple_root_dir_block(disk: &Ext2MemoryDisk, root_bid: u32) {
    let mut block = vec![0u8; BLOCK_SIZE];

    // '.'
    block[0..4].copy_from_slice(&ROOT_INO.to_le_bytes());
    block[4..6].copy_from_slice(&(12u16).to_le_bytes());
    block[6] = 1;
    block[7] = 2;
    block[8] = DOT_BYTE[0];

    // '..'
    block[12..16].copy_from_slice(&ROOT_INO.to_le_bytes());
    block[16..18].copy_from_slice(&((BLOCK_SIZE - 12) as u16).to_le_bytes());
    block[18] = 2;
    block[19] = 2;
    block[20..22].copy_from_slice(DOT_DOT_BYTE);

    disk.segment()
        .write_bytes(Bid::new(root_bid as u64).to_offset(), &block)
        .unwrap();
}

// ===========================================================================
// Layer 4: Fixture — strategy enums, Ext2Fixture, Ext2FixtureBuilder
// ===========================================================================

#[derive(Clone)]
pub(super) enum BlockBitmapInit {
    /// Marks only mandatory per-group metadata blocks.
    MetadataOnly,
    /// Marks metadata plus explicit extra blocks in group 0.
    MetadataPlus(Vec<u32>),
    /// Marks all group-0 blocks allocated for ENOSPC testing.
    Full,
}

#[derive(Clone)]
pub(super) enum InodeBitmapInit {
    /// Marks only reserved group-0 inodes `[1, first_ino)`.
    ReservedOnly,
    /// Marks reserved group-0 inodes plus explicit extra group-0 inodes.
    ReservedPlus(Vec<u32>),
    /// Marks all group-0 inodes allocated.
    Full,
}

pub(super) struct Ext2Fixture {
    pub disk: Arc<Ext2MemoryDisk>,
    pub ext2: Arc<Ext2>,
    pub sb: SuperBlock,
    pub descs: Vec<RawBlockGroup>,
}

type PreparedFixture = (
    RawSuperBlock,
    SuperBlock,
    Vec<RawBlockGroup>,
    Arc<Ext2MemoryDisk>,
    Group0Layout,
);

impl Ext2Fixture {
    pub(super) fn root(&self) -> Arc<Inode> {
        self.ext2.read_inode(ROOT_INO).unwrap()
    }
}

pub(super) struct Ext2FixtureBuilder {
    groups: u32,
    nblocks: usize,
    sb_free_blocks: Option<u32>,
    sb_free_inodes: Option<u32>,
    group0_free_blocks: Option<u16>,
    group0_free_inodes: Option<u16>,
    group0_used_dirs: Option<u16>,
    init_root: bool,
    block_bitmap: Option<BlockBitmapInit>,
    inode_bitmap: Option<InodeBitmapInit>,
    custom_device: Option<Arc<dyn BlockDevice>>,
}

impl Ext2FixtureBuilder {
    pub(super) fn new(groups: u32, nblocks: usize) -> Self {
        Self {
            groups,
            nblocks,
            sb_free_blocks: None,
            sb_free_inodes: None,
            group0_free_blocks: None,
            group0_free_inodes: None,
            group0_used_dirs: None,
            init_root: true,
            block_bitmap: None,
            inode_bitmap: None,
            custom_device: None,
        }
    }

    pub(super) fn with_free_blocks(mut self, sb_free_blocks: u32, group0_free_blocks: u16) -> Self {
        self.sb_free_blocks = Some(sb_free_blocks);
        self.group0_free_blocks = Some(group0_free_blocks);
        self
    }

    pub(super) fn with_free_inodes(mut self, sb_free_inodes: u32, group0_free_inodes: u16) -> Self {
        self.sb_free_inodes = Some(sb_free_inodes);
        self.group0_free_inodes = Some(group0_free_inodes);
        self
    }

    pub(super) fn with_group0_used_dirs(mut self, used_dirs: u16) -> Self {
        self.group0_used_dirs = Some(used_dirs);
        self
    }

    pub(super) fn block_bitmap(mut self, init: BlockBitmapInit) -> Self {
        self.block_bitmap = Some(init);
        self
    }

    pub(super) fn inode_bitmap(mut self, init: InodeBitmapInit) -> Self {
        self.inode_bitmap = Some(init);
        self
    }

    fn prepare(&self) -> Result<PreparedFixture> {
        let mut raw_sb = make_valid_raw_super_block(self.groups);
        if let Some(sb_free_blocks) = self.sb_free_blocks {
            raw_sb.free_blocks_count = sb_free_blocks;
        }
        if let Some(sb_free_inodes) = self.sb_free_inodes {
            raw_sb.free_inodes_count = sb_free_inodes;
        }

        let sb = SuperBlock::try_from(raw_sb)?;
        let mut descs = (0..sb.nr_block_groups() as usize)
            .map(|idx| make_valid_group_desc(&sb, idx))
            .collect::<Vec<_>>();

        let layout = group0_layout(&sb);
        validate_group0_layout(&sb, &layout)?;
        descs[0].block_bitmap_bid = layout.block_bitmap_bid;
        descs[0].inode_bitmap_bid = layout.inode_bitmap_bid;
        descs[0].inode_table_bid = layout.inode_table_bid;

        if let Some(v) = self.group0_free_blocks {
            descs[0].free_blocks_count = v;
        }
        if let Some(v) = self.group0_free_inodes {
            descs[0].free_inodes_count = v;
        }
        if let Some(v) = self.group0_used_dirs {
            descs[0].used_dirs_count = v;
        }

        let disk_blocks = self.nblocks.max(sb.total_blocks() as usize);
        let disk = Arc::new(Ext2MemoryDisk::new(disk_blocks));
        disk.write_super_block(&raw_sb);
        disk.write_group_desc_table(&sb, &descs);

        Ok((raw_sb, sb, descs, disk, layout))
    }

    /// Marks the per-group metadata bits that every group must have for
    /// mount-time bitmap validation to pass.
    fn mark_group_metadata_bits(
        bitmap: &mut [u8],
        sb: &SuperBlock,
        desc: &RawBlockGroup,
        group_idx: usize,
    ) {
        let first = sb.group_first_block_no(group_idx);
        let last = sb.group_last_block_no(group_idx);

        set_bit_lsb0(bitmap, (desc.block_bitmap_bid - first) as usize);
        set_bit_lsb0(bitmap, (desc.inode_bitmap_bid - first) as usize);

        let nr_inode_table_blocks_per_group = sb.nr_inode_table_blocks_per_group();
        for i in 0..nr_inode_table_blocks_per_group {
            set_bit_lsb0(bitmap, (desc.inode_table_bid + i - first) as usize);
        }

        if group_idx == 0 {
            let sb_bid = sb.bid(0);
            if sb_bid >= first && sb_bid <= last {
                set_bit_lsb0(bitmap, (sb_bid - first) as usize);
            }
            let group_desc_bid = sb.group_descriptors_bid(0);
            if group_desc_bid >= first && group_desc_bid <= last {
                set_bit_lsb0(bitmap, (group_desc_bid - first) as usize);
            }
        }
    }

    fn write_bitmaps(
        &self,
        sb: &SuperBlock,
        descs: &[RawBlockGroup],
        disk: &Ext2MemoryDisk,
        layout: &Group0Layout,
    ) {
        let root_bid = layout.first_data_bid.saturating_add(1);

        // Resolve bitmap strategies.
        let block_init = self.block_bitmap.clone().unwrap_or_else(|| {
            if self.init_root {
                BlockBitmapInit::MetadataPlus(vec![root_bid])
            } else {
                BlockBitmapInit::MetadataOnly
            }
        });
        let inode_init = self.inode_bitmap.clone().unwrap_or_else(|| {
            if self.init_root {
                InodeBitmapInit::ReservedPlus(vec![ROOT_INO])
            } else {
                InodeBitmapInit::ReservedOnly
            }
        });

        // Block bitmaps — one pass per group.
        for (group_idx, desc) in descs.iter().enumerate() {
            let first = sb.group_first_block_no(group_idx);
            let last = sb.group_last_block_no(group_idx);
            let mut bitmap_block = [0u8; BLOCK_SIZE];

            Self::mark_group_metadata_bits(&mut bitmap_block, sb, desc, group_idx);

            if group_idx == 0 {
                match &block_init {
                    BlockBitmapInit::MetadataOnly => {}
                    BlockBitmapInit::MetadataPlus(extra) => {
                        for &block in extra {
                            if block >= first && block <= last {
                                set_bit_lsb0(&mut bitmap_block, (block - first) as usize);
                            }
                        }
                    }
                    BlockBitmapInit::Full => {
                        let group_size = (last - first + 1) as usize;
                        for bit in 0..group_size {
                            set_bit_lsb0(&mut bitmap_block, bit);
                        }
                    }
                }
            }

            disk.segment()
                .write_bytes(
                    Bid::new(desc.block_bitmap_bid as u64).to_offset(),
                    &bitmap_block,
                )
                .unwrap();
        }

        // Inode bitmap — group 0 only.
        {
            let mut bitmap = [0u8; BLOCK_SIZE];
            match &inode_init {
                InodeBitmapInit::ReservedOnly => {
                    for bit in 0..(sb.first_ino() as usize).saturating_sub(1) {
                        set_bit_lsb0(&mut bitmap, bit);
                    }
                }
                InodeBitmapInit::ReservedPlus(extra) => {
                    for bit in 0..(sb.first_ino() as usize).saturating_sub(1) {
                        set_bit_lsb0(&mut bitmap, bit);
                    }
                    for &ino in extra {
                        if ino == 0 || ino > sb.nr_inodes_per_group() {
                            continue;
                        }
                        set_bit_lsb0(&mut bitmap, (ino - 1) as usize);
                    }
                }
                InodeBitmapInit::Full => {
                    for bit in 0..(sb.nr_inodes_per_group() as usize) {
                        set_bit_lsb0(&mut bitmap, bit);
                    }
                }
            }
            disk.segment()
                .write_bytes(
                    Bid::new(descs[0].inode_bitmap_bid as u64).to_offset(),
                    &bitmap,
                )
                .unwrap();
        }

        // Root directory data block (independent of bitmaps).
        if self.init_root {
            write_simple_root_dir_block(disk, root_bid);
        }
    }

    pub(super) fn build(self) -> Result<Ext2Fixture> {
        let (_, sb, descs, disk, layout) = self.prepare()?;
        self.write_bitmaps(&sb, &descs, &disk, &layout);

        let root_bid = layout.first_data_bid.saturating_add(1);
        if self.init_root {
            let root_raw = make_root_raw_inode(root_bid);
            write_raw_inode_to_disk(&sb, &descs, ROOT_INO, &root_raw, &disk);
        }

        let device: Arc<dyn BlockDevice> = self
            .custom_device
            .unwrap_or_else(|| disk.clone() as Arc<dyn BlockDevice>);
        let ext2 = Ext2::open(device, None)?;

        Ok(Ext2Fixture {
            disk,
            ext2,
            sb,
            descs,
        })
    }
}

// ===========================================================================
// Layer 5: Convenience helpers
// ===========================================================================

pub(super) fn create_file(dir: &Arc<Inode>, name: &str) -> Arc<Inode> {
    dir.create(name, InodeType::File, FilePerm::from_bits_truncate(0o644))
        .unwrap()
}

pub(super) fn default_fixture() -> (Ext2Fixture, Arc<Inode>) {
    clocks::init_for_ktest();
    let f = Ext2FixtureBuilder::new(1, 256)
        .with_free_blocks(64, 64)
        .with_free_inodes(1000, 1000)
        .with_group0_used_dirs(1)
        .build()
        .unwrap();
    let root = f.root();
    (f, root)
}
