// SPDX-License-Identifier: MPL-2.0

//! The `Ext4` filesystem object: mount, geometry, and inode lookup.
//!
//! At mount the superblock and block-group descriptors are parsed and each group
//! loads its block and inode bitmaps. `Ext4` resolves an inode number to its
//! owning group and reads the inode on demand, caching live inodes per group.
//! This is a read-only mount: there is no block/inode allocation, journaling, or
//! metadata writeback.

use device_id::DeviceId;

use super::{
    block_group::BlockGroup,
    inode::{Inode, InodeDesc},
    prelude::*,
    super_block::{RawSuperBlock, SUPER_BLOCK_OFFSET, SuperBlock},
};
use crate::fs::vfs::file_system::FsEventSubscriberStats;

/// Root directory inode number.
pub(super) const ROOT_INO: Ext4Ino = 2;

/// An ext4 filesystem instance.
pub struct Ext4 {
    block_device: Arc<dyn BlockDevice>,
    /// Superblock with dirty tracking.
    super_block: RwMutex<Dirty<SuperBlock>>,
    /// Per-group block-side metadata (descriptor + block bitmap).
    block_groups: Vec<BlockGroup>,
    /// Inodes per group, cached once at mount to avoid locking `super_block` on
    /// the inode read path.
    nr_inodes_per_group: u32,
    /// Total inode count, cached at mount like `nr_inodes_per_group` — the
    /// corrupt-ino bound check sits on every inode load, so caching it here
    /// avoids taking the `super_block` lock on that path.
    total_inodes: u32,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    self_ref: Weak<Ext4>,
}

impl Ext4 {
    /// Mounts an ext4 volume from a block device.
    pub(super) fn open(device: Arc<dyn BlockDevice>) -> Result<Arc<Self>> {
        let raw_super_block = device.read_val::<RawSuperBlock>(SUPER_BLOCK_OFFSET)?;
        let super_block = SuperBlock::try_from(raw_super_block)?;
        let nr_inodes_per_group = super_block.nr_inodes_per_group();
        let total_inodes = super_block.total_inodes();

        let block_groups = Self::load_block_groups(device.clone(), &super_block)?;

        let ext4 = Arc::new_cyclic(|weak| Ext4 {
            block_device: device,
            super_block: RwMutex::new(Dirty::new(super_block)),
            block_groups,
            nr_inodes_per_group,
            total_inodes,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            self_ref: weak.clone(),
        });

        Ok(ext4)
    }

    pub(super) fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }

    /// Returns the device ID of the backing block device.
    pub(super) fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }

    /// Loads every block group from the descriptor table, which immediately
    /// follows the block holding the superblock.
    fn load_block_groups(
        device: Arc<dyn BlockDevice>,
        super_block: &SuperBlock,
    ) -> Result<Vec<BlockGroup>> {
        let nr_groups = super_block.nr_block_groups() as usize;
        let gdt_base_offset =
            (super_block.first_data_block() as usize + 1) * super_block.block_size();

        let mut block_groups = Vec::with_capacity(nr_groups);
        for group_idx in 0..nr_groups {
            let group = BlockGroup::load(device.clone(), group_idx, super_block, gdt_base_offset)?;
            block_groups.push(group);
        }
        Ok(block_groups)
    }

    /// Returns a read guard of the superblock.
    pub(super) fn super_block(&self) -> RwMutexReadGuard<'_, Dirty<SuperBlock>> {
        self.super_block.read()
    }

    pub(super) fn block_device(&self) -> &Arc<dyn BlockDevice> {
        &self.block_device
    }
    /// Refuses a remount-time filesystem-flag change with `EOPNOTSUPP`.
    ///
    /// Ext4 processes no runtime change to the filesystem flags, so
    /// `set_fs_flags` reports the change as unsupported rather than silently
    /// answering `Ok(())` — a fake success that would pretend to honor a flag
    /// transition it never performs.
    pub(super) fn refuse_fs_flags_change(&self) -> Result<()> {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "ext4 does not support changing filesystem flags at remount"
        );
    }

    /// Submits an asynchronous read of one or more blocks starting at `bid`.
    pub(super) fn read_blocks_async(
        &self,
        bid: Ext4Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.block_device
            .read_blocks_async(Bid::new(bid), bio_segment, complete_fn, io_batch)?;
        Ok(())
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn this(&self) -> Weak<Ext4> {
        self.self_ref.clone()
    }

    /// Locates and decodes an inode's metadata directly from the inode table.
    ///
    /// The owning group performs the on-disk load; this method only routes the
    /// inode number to its group.
    pub(super) fn read_inode_desc(&self, ino: Ext4Ino) -> Result<InodeDesc> {
        self.find_group(ino)?.read_inode_desc(ino)
    }

    /// Returns the block group that owns `ino`.
    fn find_group(&self, ino: Ext4Ino) -> Result<&BlockGroup> {
        if ino == 0 {
            return_errno_with_message!(Errno::ENOENT, "invalid inode number 0");
        }
        // A corrupt dirent can carry any 32-bit ino; bound it by the
        // superblock's inode count, not just the group range (P1 review item,
        // batch-fixed at P5). Uses the mount-time cache to avoid taking the
        // `super_block` lock on every inode load.
        if ino > self.total_inodes {
            return_errno_with_message!(Errno::ENOENT, "inode number beyond s_inodes_count");
        }
        let group_idx = ((ino - 1) / self.nr_inodes_per_group) as usize;
        self.block_groups
            .get(group_idx)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "inode block group out of range"))
    }

    /// Reads the root directory's inode metadata.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn root_inode_desc(&self) -> Result<InodeDesc> {
        self.read_inode_desc(ROOT_INO)
    }

    /// Reads an inode, returning the cached `Arc<Inode>` if one already exists.
    ///
    /// Routing through the owning group's inode cache gives every reader of one
    /// inode number the same in-memory inode (identity), so concurrent readers
    /// share a single object.
    pub(super) fn read_inode(&self, ino: Ext4Ino) -> Result<Arc<Inode>> {
        self.find_group(ino)?
            .lookup_inode(ino, self.self_ref.clone())
    }

    /// Reads the root directory inode.
    pub(super) fn root_inode(&self) -> Result<Arc<Inode>> {
        self.read_inode(ROOT_INO)
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{
        super::test_utils::{Ext4FixtureBuilder, make_empty_file_inode, make_file_inode},
        *,
    };

    #[ktest]
    fn mount_and_read_root() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let root = f.ext4.root_inode_desc().unwrap();
        assert_eq!(root.type_(), InodeType::Dir);
        assert_eq!(root.link_count(), 2);
        assert!(root.is_extent_based());
        assert_eq!(f.ext4.super_block().nr_block_groups(), 1);
    }

    #[ktest]
    fn read_inode_zero_fails() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        assert!(f.ext4.read_inode_desc(0).is_err());
    }

    /// `statfs` reports usable capacity (raw blocks minus static metadata
    /// overhead — a read-only mount consults no journal), a reserved-block
    /// adjusted `bavail`, and a UUID-derived `fsid`.
    #[ktest]
    fn statfs_reports_usable_space() {
        use crate::fs::vfs::file_system::FileSystem;

        // 3 groups of 2048 blocks, non-zero reserved blocks and UUID. The fixture
        // sets `first_data_block == 0`, `sparse_super`, and no reserved GDT, so
        // only groups 0 and 1 carry a superblock/GDT copy.
        let uuid = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
            0x77, 0x88,
        ];
        let f = Ext4FixtureBuilder::new(2048, 256, 3 * 2048)
            .with_reserved_blocks(500)
            .with_uuid(uuid)
            .build()
            .unwrap();
        let sb = f.ext4.sb();

        // Overhead = 3 groups * (16 inode-table + 2 bitmaps) + 2 super-bearing
        // groups * (1 super + 1 GDT) = 58 blocks. A read-only mount adds no
        // journal-log overhead (the journal is never loaded).
        let expected_overhead = 3 * (16 + 2) + 2 * (1 + 1);
        assert_eq!(sb.blocks, 3 * 2048 - expected_overhead);

        // The read-only fixture declares no free blocks, so `bfree` is zero and
        // `bavail` saturates at zero after subtracting the 500 reserved blocks.
        assert_eq!(sb.bfree, 0);
        assert_eq!(sb.bavail, 0);
        // `fsid` is the UUID's low 8 bytes.
        assert_eq!(sb.fsid, u64::from_le_bytes(uuid[..8].try_into().unwrap()));
    }

    /// A remount that asks to change filesystem flags is refused with
    /// `EOPNOTSUPP` (ext4 honors no runtime change), not silently accepted.
    #[ktest]
    fn set_fs_flags_refuses_loudly() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let err = f.ext4.refuse_fs_flags_change().unwrap_err();
        assert_eq!(err.error(), Errno::EOPNOTSUPP);
    }

    #[ktest]
    fn read_small_file_end_to_end() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let data_block = 100u32;
        let content = b"hello ext4 phase 1 read path!";
        f.write_data_block(data_block, content);
        f.write_raw_inode(11, &make_file_inode(data_block, content.len() as u32));

        let inode = f.ext4.read_inode(11).unwrap();
        assert_eq!(inode.inode_type(), InodeType::File);
        assert_eq!(inode.size(), content.len());

        let mut buf = vec![0u8; content.len()];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        let read = inode.read_at(0, &mut writer).unwrap();
        assert_eq!(read, content.len());
        assert_eq!(&buf[..], content);
    }

    /// Reading the same inode number twice returns the same cached `Arc`
    /// (identity), while a different inode number is a distinct object.
    #[ktest]
    fn read_inode_returns_same_arc_identity() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        f.write_raw_inode(11, &make_empty_file_inode());

        let a = f.ext4.read_inode(11).unwrap();
        let b = f.ext4.read_inode(11).unwrap();
        assert!(Arc::ptr_eq(&a, &b));

        // A different inode is a different identity.
        f.write_raw_inode(12, &make_empty_file_inode());
        let c = f.ext4.read_inode(12).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
    }

    /// Mount contract (report §4.5): a corrupt dirent can carry any 32-bit ino;
    /// both the inode-count bound and the group-range check must answer ENOENT,
    /// never panic.
    #[ktest]
    fn read_inode_rejects_out_of_range_ino() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        for ino in [u32::MAX, 1 << 20] {
            let Err(err) = f.ext4.read_inode(ino) else {
                panic!("out-of-range ino {ino} must not resolve");
            };
            assert_eq!(err.error(), Errno::ENOENT);
        }
    }
}
