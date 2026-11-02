// SPDX-License-Identifier: MPL-2.0

//! Block and node allocation, and on-disk metadata updates.
//!
//! The read-only driver keeps allocations simple: new blocks are taken in
//! place from segments recorded in the SIT, and every change is written
//! through to the active NAT/SIT/SSA copies. There is no log-structured
//! relocation, no garbage collection, and no checkpoint rewrite; the
//! checkpoint loaded at mount time remains valid because all metadata areas
//! are updated in place.
//!
//! # On-disk updates
//!
//! | Metadata | Location |
//! |----------|----------|
//! | NAT entry | active NAT block selected by the checkpoint NAT bitmap |
//! | SIT entry | active SIT block selected by the checkpoint SIT bitmap |
//! | Block summary | SSA block of the segment (`ssa_blkaddr + segno`) |
//! | Node footer | last 24 bytes of every node block |

use aster_block::BlockDevice;

use super::{
    fs::{Block, F2fs},
    ondisk::{
        F2FS_SUPER_MAGIC, NAT_ENTRY_PER_BLOCK, SUMMARY_FOOTER_OFFSET, SuperBlock, crc32_le,
        read_u16, read_u32, test_bit_be,
    },
    prelude::*,
};

/// Size of an on-disk SIT entry (`vblocks` + 512-bit map + `mtime`).
pub(super) const SIT_ENTRY_SIZE: usize = 74;

/// SIT entries per block (`4096 / 74`).
pub(super) const SIT_ENTRY_PER_BLOCK: usize = 55;

/// Number of blocks in one segment for the supported block size.
pub(super) const BLOCKS_PER_SEG: u32 = 512;

/// Allocation type stored in the upper bits of `f2fs_sit_entry.vblocks`.
pub(super) const CURSEG_HOT_DATA: u16 = 0;

/// Allocation type for node segments.
pub(super) const CURSEG_HOT_NODE: u16 = 3;

const SIT_VBLOCKS_MASK: u16 = 0x03ff;
const SIT_TYPE_SHIFT: u16 = 10;

/// Size of one SIT entry block bitmap in bytes.
pub(super) const SIT_MAP_SIZE: usize = 64;

/// The kind of block being allocated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BlockKind {
    Data,
    Node,
}

impl BlockKind {
    fn curseg_type(self) -> u16 {
        match self {
            BlockKind::Data => CURSEG_HOT_DATA,
            BlockKind::Node => CURSEG_HOT_NODE,
        }
    }

    fn summary_type(self) -> u8 {
        match self {
            BlockKind::Data => 0,
            BlockKind::Node => 1,
        }
    }
}

/// In-memory copy of one SIT entry.
#[derive(Clone, Copy, Debug)]
pub(super) struct SitEntry {
    pub vblocks: u16,
    pub valid_map: [u8; SIT_MAP_SIZE],
    pub mtime: u64,
}

impl SitEntry {
    fn valid_count(self) -> u16 {
        self.vblocks & SIT_VBLOCKS_MASK
    }

    fn segment_type(self) -> u16 {
        self.vblocks >> SIT_TYPE_SHIFT
    }

    fn has_free_block(self) -> bool {
        self.valid_count() < BLOCKS_PER_SEG as u16
    }

    fn is_free(self) -> bool {
        self.valid_count() == 0
    }
}

/// Mutable allocation metadata, guarded by a filesystem-wide mutex.
pub(super) struct WriteState {
    pub sit: Vec<SitEntry>,
    pub sit_bitmap: Vec<u8>,
    pub sit_blocks: u32,
    pub next_free_nid: u32,
    pub valid_inode_count: u32,
    /// Segments that the checkpoint names as current segments.
    ///
    /// Their summary state lives in the checkpoint pack, so allocating or
    /// freeing arbitrary blocks in them would make other F2FS implementations
    /// reject the volume at mount time. The allocator skips them.
    pub excluded: Vec<bool>,
    pub next_data_seg: usize,
    pub next_node_seg: usize,
    pub free_blocks: usize,
}

impl F2fs {
    /// Returns the mutable allocation state.
    pub(super) fn write_state(&self) -> MutexGuard<'_, WriteState> {
        self.alloc_state.lock()
    }

    /// Loads the SIT area and initializes the allocator.
    pub(super) fn load_write_state(
        super_block: &SuperBlock,
        checkpoint: &super::ondisk::Checkpoint,
        device: &Arc<dyn BlockDevice>,
    ) -> Result<WriteState> {
        let main_segments = super_block.segment_count_main as usize;
        let blocks_per_seg = super_block.blocks_per_seg();
        let sit_blocks = super_block.segment_count_sit * blocks_per_seg / 2;

        let mut sit = Vec::with_capacity(main_segments);
        let mut cache: BTreeMap<u32, Block> = BTreeMap::new();
        for segno in 0..main_segments as u32 {
            let block_addr = sit_block_addr(super_block, &checkpoint.sit_bitmap, sit_blocks, segno);
            let block = match cache.get(&block_addr) {
                Some(block) => block.clone(),
                None => {
                    let block = super::fs::read_device_block(device, block_addr)?;
                    cache.insert(block_addr, block.clone());
                    block
                }
            };
            let offset = (segno as usize % SIT_ENTRY_PER_BLOCK) * SIT_ENTRY_SIZE;
            let mut valid_map = [0u8; SIT_MAP_SIZE];
            valid_map.copy_from_slice(&block[offset + 2..offset + SIT_MAP_SIZE + 2]);
            sit.push(SitEntry {
                vblocks: read_u16(block.as_slice(), offset),
                valid_map,
                mtime: u64::from_le_bytes(
                    block[offset + SIT_MAP_SIZE + 2..offset + SIT_ENTRY_SIZE]
                        .try_into()
                        .unwrap(),
                ),
            });
        }

        let mut excluded = vec![false; main_segments];
        for segno in checkpoint
            .cur_node_segno
            .iter()
            .chain(checkpoint.cur_data_segno.iter())
        {
            if (*segno as usize) < main_segments {
                excluded[*segno as usize] = true;
            }
        }

        // F2FS reserves part of the main area for over-provisioning. Only the
        // blocks below `user_block_count` are available for user data.
        let valid_blocks: u64 = sit.iter().map(|entry| entry.valid_count() as u64).sum();
        let allocatable_free: usize = sit
            .iter()
            .enumerate()
            .filter(|(segno, _)| !excluded[*segno])
            .map(|(_, entry)| {
                (BLOCKS_PER_SEG as usize).saturating_sub(entry.valid_count() as usize)
            })
            .sum();
        let free_blocks =
            allocatable_free.min(checkpoint.user_block_count.saturating_sub(valid_blocks) as usize);

        Ok(WriteState {
            sit,
            sit_bitmap: checkpoint.sit_bitmap.clone(),
            sit_blocks,
            next_free_nid: checkpoint.next_free_nid,
            valid_inode_count: checkpoint.valid_inode_count,
            excluded,
            next_data_seg: 0,
            next_node_seg: 0,
            free_blocks,
        })
    }

    /// Allocates a free main-area block of the requested kind.
    pub(super) fn alloc_block(&self, state: &mut WriteState, kind: BlockKind) -> Result<u32> {
        if state.free_blocks == 0 {
            return_errno_with_message!(Errno::ENOSPC, "the F2FS volume is full");
        }
        let super_block = self.super_block();
        let want_type = kind.curseg_type();
        let segment_count = state.sit.len();
        let hint = match kind {
            BlockKind::Data => state.next_data_seg,
            BlockKind::Node => state.next_node_seg,
        };

        // Prefer segments that already hold blocks of the same kind so that a
        // segment summary never mixes node and data entries.
        let mut chosen = None;
        for pass in 0..2 {
            for step in 0..segment_count {
                let segno = (hint + step) % segment_count;
                if state.excluded[segno] {
                    continue;
                }
                let entry = state.sit[segno];
                if !entry.has_free_block() {
                    continue;
                }
                let matches = entry.segment_type() == want_type;
                if pass == 0 && !matches {
                    continue;
                }
                chosen = Some(segno);
                break;
            }
            if chosen.is_some() {
                break;
            }
        }

        let segno =
            chosen.ok_or_else(|| Error::with_message(Errno::ENOSPC, "no free F2FS segment"))?;
        let entry = &mut state.sit[segno];
        let block_off = (0..BLOCKS_PER_SEG as usize)
            .find(|bit| !test_bit_be(&entry.valid_map, *bit))
            .ok_or_else(|| Error::with_message(Errno::ENOSPC, "no free block in segment"))?;

        entry.valid_map[block_off / 8] |= 1 << (7 - (block_off % 8));
        let count = entry.valid_count().saturating_add(1);
        let seg_type = if entry.is_free() {
            want_type
        } else {
            entry.segment_type()
        };
        entry.vblocks = (seg_type << SIT_TYPE_SHIFT) | (count & SIT_VBLOCKS_MASK);
        entry.mtime = 1;

        let entry_snapshot = *entry;
        write_sit_entry(self, state, segno, entry_snapshot)?;
        state.free_blocks = state.free_blocks.saturating_sub(1);
        match kind {
            BlockKind::Data => state.next_data_seg = (segno + 1) % segment_count,
            BlockKind::Node => state.next_node_seg = (segno + 1) % segment_count,
        }

        let block_addr =
            super_block.main_blkaddr + segno as u32 * BLOCKS_PER_SEG + block_off as u32;
        Ok(block_addr)
    }

    /// Frees a main-area block.
    pub(super) fn free_block(&self, state: &mut WriteState, block_addr: u32) -> Result<()> {
        let super_block = self.super_block();
        let main_start = super_block.main_blkaddr;
        let main_blocks = super_block.segment_count_main * BLOCKS_PER_SEG;
        if block_addr < main_start || block_addr >= main_start + main_blocks {
            return_errno_with_message!(Errno::EIO, "invalid F2FS block to free");
        }

        let relative = block_addr - main_start;
        let segno = (relative / BLOCKS_PER_SEG) as usize;
        let block_off = (relative % BLOCKS_PER_SEG) as usize;
        let entry = &mut state.sit[segno];
        if !test_bit_be(&entry.valid_map, block_off) {
            return Ok(());
        }

        entry.valid_map[block_off / 8] &= !(1 << (7 - (block_off % 8)));
        let seg_type = entry.segment_type();
        let count = entry.valid_count().saturating_sub(1);
        entry.vblocks = (seg_type << SIT_TYPE_SHIFT) | (count & SIT_VBLOCKS_MASK);
        entry.mtime = 1;

        let snapshot = *entry;
        write_sit_entry(self, state, segno, snapshot)?;
        // Blocks in the checkpoint's current segments are not allocatable, so
        // freeing one does not increase the usable free-block budget.
        if !state.excluded[segno] {
            state.free_blocks = state.free_blocks.saturating_add(1);
        }
        Ok(())
    }

    /// Writes a block summary entry and the summary footer type.
    pub(super) fn write_summary(
        &self,
        block_addr: u32,
        nid: u32,
        ofs_in_node: u16,
        kind: BlockKind,
    ) -> Result<()> {
        let super_block = self.super_block();
        let relative = block_addr.saturating_sub(super_block.main_blkaddr);
        let segno = relative / BLOCKS_PER_SEG;
        let block_off = (relative % BLOCKS_PER_SEG) as usize;
        let summary_addr = super_block.ssa_blkaddr + segno;

        let mut block = self.read_block(summary_addr)?;
        let offset = block_off * 7;
        block[offset..offset + 4].copy_from_slice(&nid.to_le_bytes());
        block[offset + 4] = 0;
        block[offset + 5..offset + 7].copy_from_slice(&ofs_in_node.to_le_bytes());
        block[SUMMARY_FOOTER_OFFSET] = kind.summary_type();
        self.write_block(summary_addr, &block)
    }

    /// Writes a NAT entry to the active NAT copy.
    pub(super) fn write_nat_entry(&self, nid: u32, ino: u32, block_addr: u32) -> Result<()> {
        let nat_block_addr = self.nat_block_addr(nid)?;
        let mut block = self.read_block(nat_block_addr)?;
        let offset = (nid as usize % NAT_ENTRY_PER_BLOCK) * 9;
        block[offset] = 0;
        block[offset + 1..offset + 5].copy_from_slice(&ino.to_le_bytes());
        block[offset + 5..offset + 9].copy_from_slice(&block_addr.to_le_bytes());
        self.write_block(nat_block_addr, &block)
    }

    /// Finds a free node id and records a NAT entry for it.
    ///
    /// `owner_ino` is the inode that owns the node. For inode nodes the node
    /// id and the inode number are the same, so `owner_ino` equals the nid.
    pub(super) fn alloc_node(
        &self,
        state: &mut WriteState,
        owner_ino: u32,
        nid: Option<u32>,
    ) -> Result<(u32, u32)> {
        let nid = match nid {
            Some(nid) => nid,
            None => self.find_free_nid(state)?,
        };
        let block_addr = self.alloc_block(state, BlockKind::Node)?;
        self.write_nat_entry(nid, owner_ino, block_addr)?;
        self.write_summary(block_addr, nid, 0, BlockKind::Node)?;
        Ok((nid, block_addr))
    }

    /// Frees a node id.
    pub(super) fn free_nid(&self, nid: u32) -> Result<()> {
        self.write_nat_entry(nid, 0, 0)
    }

    /// Updates the checkpoint counters and checksum in place.
    ///
    /// The driver does not write a new checkpoint pack, but keeping the
    /// counters and CRC consistent lets other F2FS implementations mount the
    /// volume after a clean `sync`.
    pub(super) fn flush_checkpoint(&self) -> Result<()> {
        let state = self.alloc_state.lock();
        let mut block = self.read_block(self.checkpoint_address)?;

        let mut valid_blocks = 0u64;
        let mut node_blocks = 0u64;
        let mut free_segments = 0u32;
        for entry in &state.sit {
            let count = entry.valid_count() as u64;
            valid_blocks += count;
            if count == 0 {
                free_segments += 1;
            }
            if matches!(entry.segment_type(), 3..=5) {
                node_blocks += count;
            }
        }

        block[16..24].copy_from_slice(&valid_blocks.to_le_bytes());
        block[32..36].copy_from_slice(&free_segments.to_le_bytes());
        block[144..148].copy_from_slice(&(node_blocks as u32).to_le_bytes());
        block[148..152].copy_from_slice(&state.valid_inode_count.to_le_bytes());
        block[152..156].copy_from_slice(&state.next_free_nid.to_le_bytes());

        let checksum_offset = read_u32(block.as_slice(), 164) as usize;
        if !(192..=super::ondisk::F2FS_BLKSIZE - 4).contains(&checksum_offset) {
            return_errno_with_message!(Errno::EIO, "invalid F2FS checkpoint checksum offset");
        }
        let checksum = crc32_le(F2FS_SUPER_MAGIC, &block[..checksum_offset]);
        block[checksum_offset..checksum_offset + 4].copy_from_slice(&checksum.to_le_bytes());

        self.write_block(self.checkpoint_address, &block)
    }

    /// Finds an unused node id starting at the checkpoint hint.
    pub(super) fn find_free_nid(&self, state: &mut WriteState) -> Result<u32> {
        let max_nid = (self.checkpoint().nat_bitmap.len() * 8 * NAT_ENTRY_PER_BLOCK) as u32;
        let start = state.next_free_nid.max(3);
        for pass in 0..2 {
            let (from, to) = if pass == 0 {
                (start, max_nid)
            } else {
                (3, start.min(max_nid))
            };
            for nid in from..to {
                if self.nat_entry_is_free(nid)? {
                    state.next_free_nid = nid + 1;
                    return Ok(nid);
                }
            }
        }
        return_errno_with_message!(Errno::ENOSPC, "no free F2FS node id");
    }

    fn nat_entry_is_free(&self, nid: u32) -> Result<bool> {
        let nat_block_addr = self.nat_block_addr(nid)?;
        let block = self.read_block(nat_block_addr)?;
        let offset = (nid as usize % NAT_ENTRY_PER_BLOCK) * 9;
        let ino = read_u32(block.as_slice(), offset + 1);
        let block_addr = read_u32(block.as_slice(), offset + 5);
        Ok(ino == 0 && block_addr == 0)
    }

    fn nat_block_addr(&self, nid: u32) -> Result<u32> {
        let block_off = nid as usize / NAT_ENTRY_PER_BLOCK;
        if block_off >= self.checkpoint().nat_bitmap.len() * 8 {
            return_errno_with_message!(Errno::EIO, "the F2FS node id is out of range");
        }
        Ok(self.nat_block_address(block_off))
    }
}

/// Computes the active SIT block for a segment.
fn sit_block_addr(super_block: &SuperBlock, sit_bitmap: &[u8], sit_blocks: u32, segno: u32) -> u32 {
    let offset = segno / SIT_ENTRY_PER_BLOCK as u32;
    let mut addr = super_block.sit_blkaddr + offset;
    if test_bit_be(sit_bitmap, offset as usize) {
        addr += sit_blocks;
    }
    addr
}

/// Writes one SIT entry back to its active copy.
fn write_sit_entry(fs: &F2fs, state: &WriteState, segno: usize, entry: SitEntry) -> Result<()> {
    let super_block = fs.super_block();
    let block_addr = sit_block_addr(
        super_block,
        &state.sit_bitmap,
        state.sit_blocks,
        segno as u32,
    );
    let mut block = fs.read_block(block_addr)?;
    let offset = (segno % SIT_ENTRY_PER_BLOCK) * SIT_ENTRY_SIZE;
    block[offset..offset + 2].copy_from_slice(&entry.vblocks.to_le_bytes());
    block[offset + 2..offset + 2 + SIT_MAP_SIZE].copy_from_slice(&entry.valid_map);
    block[offset + SIT_MAP_SIZE + 2..offset + SIT_ENTRY_SIZE]
        .copy_from_slice(&entry.mtime.to_le_bytes());
    fs.write_block(block_addr, &block)
}
