// SPDX-License-Identifier: MPL-2.0

//! Core filesystem state for a mounted F2FS volume.
//!
//! This driver understands the on-disk layout produced by `mkfs.f2fs`:
//! the superblock, the checkpoint (including the NAT journal), the NAT and
//! SIT areas, node blocks, and both ways F2FS stores directory entries
//! (regular dentry blocks and inline dentries).
//!
//! Reads go straight to the block device. Writes are applied in place: the
//! driver allocates blocks from the SIT, updates NAT/SIT/SSA metadata
//! immediately, and leaves the checkpoint untouched (it remains valid because
//! all mutable metadata areas are updated in place). There is no garbage
//! collection or log-structured relocation yet.
//!
//! # Locking
//!
//! Allocation state is guarded by [`F2fs::alloc_state`]. Inodes hold a weak
//! reference back to the filesystem and guard their own mutable state with an
//! `RwMutex`. Write paths acquire the allocation lock first and then the
//! inode lock; read paths only take the inode lock.

use core::sync::atomic::Ordering;

use aster_block::{BlockDevice, SECTOR_SIZE};
use device_id::DeviceId;
use ostd::mm::VmIo;

use super::{
    alloc::WriteState,
    inode::Inode,
    ondisk::{
        Checkpoint, F2FS_BLKSIZE, NAT_ENTRY_PER_BLOCK, NatEntry, SUM_TYPE_NODE,
        SUMMARY_FOOTER_OFFSET, SUMMARY_JOURNAL_OFFSET, SuperBlock, parse_nat_entry, read_u16,
        read_u32, test_bit_be,
    },
    prelude::*,
};
use crate::fs::vfs::file_system::{AtomicFsFlags, FsEventSubscriberStats, FsFlags};

/// A heap-allocated 4 KiB block.
pub(super) type Block = Box<[u8; F2FS_BLKSIZE]>;

/// Allocates a zeroed block buffer.
pub(super) fn zeroed_block() -> Block {
    Box::new([0u8; F2FS_BLKSIZE])
}

/// Top-level handle for a mounted F2FS volume.
#[derive(Debug)]
pub(crate) struct F2fs {
    block_device: Arc<dyn BlockDevice>,
    super_block: SuperBlock,
    checkpoint: Checkpoint,
    pub(super) checkpoint_address: u32,
    nat_journal: BTreeMap<u32, NatEntry>,
    pub(super) alloc_state: Mutex<WriteState>,
    inodes: RwLock<BTreeMap<u32, Weak<Inode>>>,
    flags: AtomicFsFlags,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    self_ref: Weak<F2fs>,
}

impl Debug for WriteState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WriteState")
            .field("segments", &self.sit.len())
            .field("free_blocks", &self.free_blocks)
            .field("next_free_nid", &self.next_free_nid)
            .finish()
    }
}

impl F2fs {
    /// Mounts the F2FS volume stored on `block_device`.
    pub(crate) fn open(
        block_device: Arc<dyn BlockDevice>,
        flags: FsFlags,
        _args: Option<&str>,
    ) -> Result<Arc<Self>> {
        let block0 = read_device_block(&block_device, 0).inspect_err(|err| {
            error!("f2fs: failed to read the superblock: {:?}", err);
        })?;
        let super_block = SuperBlock::parse(block0.as_slice()).inspect_err(|err| {
            error!("f2fs: failed to parse the superblock: {:?}", err);
        })?;

        let device_size = block_device.metadata().nr_sectors * SECTOR_SIZE;
        let fs_size = (super_block.block_count as usize)
            .checked_mul(F2FS_BLKSIZE)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid F2FS device size"))?;
        if fs_size > device_size {
            return_errno_with_message!(
                Errno::EINVAL,
                "the F2FS volume is larger than the block device"
            );
        }

        let (checkpoint, checkpoint_address) = select_checkpoint(&block_device, &super_block)
            .inspect_err(|err| {
                error!("f2fs: failed to select a checkpoint: {:?}", err);
            })?;
        let nat_journal = load_nat_journal(&block_device, super_block.cp_blkaddr, &checkpoint)
            .inspect_err(|err| {
                error!("f2fs: failed to load the NAT journal: {:?}", err);
            })?;
        let write_state = Self::load_write_state(&super_block, &checkpoint, &block_device)
            .inspect_err(|err| {
                error!("f2fs: failed to load the SIT: {:?}", err);
            })?;

        let fs = Arc::new_cyclic(|weak_self| Self {
            block_device,
            super_block,
            checkpoint,
            checkpoint_address,
            nat_journal,
            alloc_state: Mutex::new(write_state),
            inodes: RwLock::new(BTreeMap::new()),
            flags: AtomicFsFlags::new(flags),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            self_ref: weak_self.clone(),
        });

        // Fail the mount early if the root inode cannot be parsed.
        let _ = Inode::load(&fs, fs.super_block.root_ino).inspect_err(|err| {
            error!("f2fs: failed to load the root inode: {:?}", err);
        })?;

        Ok(fs)
    }

    /// Returns the weak self reference handed to inodes.
    pub(super) fn self_ref(&self) -> Weak<F2fs> {
        self.self_ref.clone()
    }

    /// Returns the parsed superblock.
    pub(super) fn super_block(&self) -> &SuperBlock {
        &self.super_block
    }

    /// Returns the active checkpoint.
    pub(super) fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    /// Returns the device id of the container block device.
    pub(crate) fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }

    /// Returns the recorded VFS filesystem flags.
    pub(crate) fn fs_flags(&self) -> FsFlags {
        self.flags.load(Ordering::Relaxed)
    }

    /// Sets the VFS filesystem flags.
    pub(crate) fn set_fs_flags(&self, flags: FsFlags) {
        self.flags.store(flags, Ordering::Relaxed);
    }

    /// Returns whether the volume is mounted read-only.
    pub(super) fn is_read_only(&self) -> bool {
        self.fs_flags().contains(FsFlags::RDONLY)
    }

    /// Returns the FS event subscriber statistics.
    pub(crate) fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }

    /// Returns the cached inode for `ino`, if it is still alive.
    pub(super) fn cached_inode(&self, ino: u32) -> Option<Arc<Inode>> {
        self.inodes.read().get(&ino).and_then(|weak| weak.upgrade())
    }

    /// Caches a newly loaded inode.
    pub(super) fn cache_inode(&self, ino: u32, inode: Weak<Inode>) {
        self.inodes.write().insert(ino, inode);
    }

    /// Returns the root inode of the volume.
    pub(crate) fn root_inode(&self) -> Result<Arc<Inode>> {
        Inode::load(
            &self.self_ref.upgrade().expect("the filesystem is gone"),
            self.super_block.root_ino,
        )
    }

    /// Reads the block at `addr`.
    pub(super) fn read_block(&self, addr: u32) -> Result<Block> {
        read_device_block(&self.block_device, addr)
    }

    /// Writes `block` to `addr`.
    pub(super) fn write_block(&self, addr: u32, block: &Block) -> Result<()> {
        let super_block = &self.super_block;
        let max_blocks = super_block.block_count;
        if addr == 0 || addr as u64 >= max_blocks {
            return_errno_with_message!(Errno::EIO, "invalid F2FS block address");
        }
        let offset = (addr as usize)
            .checked_mul(F2FS_BLKSIZE)
            .ok_or_else(|| Error::with_message(Errno::EIO, "invalid F2FS block offset"))?;
        self.block_device
            .write_bytes(offset, block.as_slice())
            .map_err(|_| Error::with_message(Errno::EIO, "failed to write an F2FS block"))
    }

    /// Reads the node block whose node id is `nid`.
    pub(super) fn read_node(&self, nid: u32) -> Result<Block> {
        let entry = self.lookup_nat(nid)?;
        let block = self.read_block(entry.block_addr)?;

        // A node block carries its node id in the footer. Verifying it
        // detects stale NAT entries and corrupted address mappings.
        let footer_nid = read_u32(block.as_slice(), super::ondisk::NODE_FOOTER_OFFSET);
        if footer_nid != nid {
            return_errno_with_message!(
                Errno::EIO,
                "the F2FS node footer does not match the requested node id"
            );
        }

        Ok(block)
    }

    /// Looks up a NAT entry, preferring entries recorded in the checkpoint's
    /// NAT journal.
    pub(super) fn lookup_nat(&self, nid: u32) -> Result<NatEntry> {
        if let Some(entry) = self.nat_journal.get(&nid) {
            return Ok(*entry);
        }

        let block_off = nid as usize / NAT_ENTRY_PER_BLOCK;
        if block_off >= self.checkpoint.nat_bitmap.len() * 8 {
            return_errno_with_message!(Errno::EIO, "the F2FS node id is out of range");
        }

        let block = self.read_block(self.nat_block_address(block_off))?;
        let entry = parse_nat_entry(block.as_slice(), nid as usize % NAT_ENTRY_PER_BLOCK);
        // `entry.ino` is the inode that owns the node, which only equals the
        // node id for inode nodes. A zero inode number marks an unused entry.
        if entry.ino == 0 || entry.block_addr == 0 {
            return_errno_with_message!(Errno::EIO, "unused F2FS NAT entry");
        }

        Ok(entry)
    }

    /// Returns the address of the active NAT block for `block_off`.
    pub(super) fn nat_block_address(&self, block_off: usize) -> u32 {
        let blocks_per_seg = self.super_block.blocks_per_seg();
        let mut block_addr = self.super_block.nat_blkaddr + ((block_off as u32) << 1)
            - ((block_off as u32) & (blocks_per_seg - 1));
        if test_bit_be(&self.checkpoint.nat_bitmap, block_off) {
            block_addr += blocks_per_seg;
        }
        block_addr
    }

    /// Returns the number of user data blocks.
    pub(super) fn user_block_count(&self) -> usize {
        self.checkpoint.user_block_count as usize
    }

    /// Returns the number of free blocks tracked by the allocator.
    pub(super) fn free_blocks(&self) -> usize {
        self.alloc_state.lock().free_blocks
    }

    /// Returns an upper bound of the inode count supported by the NAT area.
    pub(super) fn max_inodes(&self) -> usize {
        (self.checkpoint.nat_bitmap.len() * 8).saturating_mul(NAT_ENTRY_PER_BLOCK)
    }
}

/// Reads one block directly from the device.
pub(super) fn read_device_block(block_device: &Arc<dyn BlockDevice>, addr: u32) -> Result<Block> {
    if addr == u32::MAX {
        return_errno_with_message!(Errno::EIO, "invalid F2FS block address");
    }
    let offset = (addr as usize)
        .checked_mul(F2FS_BLKSIZE)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid F2FS block offset"))?;
    let mut block = zeroed_block();
    block_device
        .read_bytes(offset, block.as_mut_slice())
        .map_err(|_| Error::with_message(Errno::EIO, "failed to read an F2FS block"))?;
    Ok(block)
}

/// Selects the newest valid checkpoint among the two checkpoint packs.
fn select_checkpoint(
    block_device: &Arc<dyn BlockDevice>,
    sb: &SuperBlock,
) -> Result<(Checkpoint, u32)> {
    let second_pack = sb
        .cp_blkaddr
        .checked_add(sb.blocks_per_seg())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid F2FS checkpoint area"))?;

    let first = read_device_block(block_device, sb.cp_blkaddr)
        .and_then(|block| Checkpoint::parse(block.as_slice()));
    let second = read_device_block(block_device, second_pack)
        .and_then(|block| Checkpoint::parse(block.as_slice()));

    match (first, second) {
        (Ok(first), Ok(second)) => {
            if first.version >= second.version {
                Ok((first, sb.cp_blkaddr))
            } else {
                Ok((second, second_pack))
            }
        }
        (Ok(first), Err(_)) => Ok((first, sb.cp_blkaddr)),
        (Err(_), Ok(second)) => Ok((second, second_pack)),
        (Err(err), Err(_)) => Err(err),
    }
}

/// Loads NAT entries that were not yet flushed to the NAT area from the
/// checkpoint's node summary blocks.
fn load_nat_journal(
    block_device: &Arc<dyn BlockDevice>,
    cp_blkaddr: u32,
    checkpoint: &Checkpoint,
) -> Result<BTreeMap<u32, NatEntry>> {
    let mut journal = BTreeMap::new();
    let max_entries = (F2FS_BLKSIZE - 5 - SUMMARY_JOURNAL_OFFSET - 2) / 13;

    for index in 1..checkpoint.cp_pack_total_block_count {
        let Some(addr) = cp_blkaddr.checked_add(index) else {
            break;
        };
        let block = read_device_block(block_device, addr)?;
        if block[SUMMARY_FOOTER_OFFSET] != SUM_TYPE_NODE {
            continue;
        }

        let count = read_u16(block.as_slice(), SUMMARY_JOURNAL_OFFSET) as usize;
        if count > max_entries {
            continue;
        }

        for entry_index in 0..count {
            let offset = SUMMARY_JOURNAL_OFFSET + 2 + entry_index * 13;
            let nid = read_u32(block.as_slice(), offset);
            let ino = read_u32(block.as_slice(), offset + 5);
            let block_addr = read_u32(block.as_slice(), offset + 9);
            if nid != 0 && ino != 0 && block_addr != 0 {
                journal.insert(nid, NatEntry { ino, block_addr });
            }
        }
    }

    Ok(journal)
}
