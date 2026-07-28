// SPDX-License-Identifier: MPL-2.0

//! On-disk inode fields and their validated in-memory representation.

use device_id::{decode_device_numbers, encode_device_numbers};

use super::{super::prelude::*, RAW_BLOCK_PTRS_LEN};

/// File permission bits (the low 12 bits of `i_mode`).
#[derive(Clone, Copy, Debug)]
pub struct FilePerm(u16);

impl FilePerm {
    /// Constructs a `FilePerm` from raw mode bits, keeping only the low 12.
    pub(in super::super) fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & 0o7777)
    }

    /// Returns the raw permission bits.
    pub(in super::super) const fn bits(&self) -> u16 {
        self.0
    }
}

bitflags! {
    /// Inode flags (`i_flags`).
    pub(in super::super) struct FileFlags: u32 {
        const SECURE_DEL = 1 << 0;
        const UNDELETE = 1 << 1;
        const COMPRESS = 1 << 2;
        const SYNC = 1 << 3;
        const IMMUTABLE = 1 << 4;
        const APPEND = 1 << 5;
        const NODUMP = 1 << 6;
        const NOATIME = 1 << 7;
        /// Directory uses an htree hash index.
        const INDEX = 1 << 12;
        /// `i_blocks` is counted in filesystem blocks, not 512-byte sectors.
        const HUGE_FILE = 1 << 18;
        /// `i_block` holds an extent tree (`EXT4_EXTENTS_FL`).
        const EXTENTS = 1 << 19;
        const EA_INODE = 1 << 21;
        /// The inode has inline data.
        const INLINE_DATA = 1 << 28;
    }
}

/// Validated, Rust-typed in-memory inode metadata.
///
/// The raw `i_block` bytes are retained in `block` for type-specific decoding.
#[derive(Clone, Debug)]
pub(in super::super) struct InodeDesc {
    type_: InodeType,
    perm: FilePerm,
    uid: u32,
    gid: u32,
    size: u64,
    atime: Duration,
    ctime: Duration,
    mtime: Duration,
    crtime: Duration,
    dtime: Duration,
    link_count: u16,
    /// `i_blocks` in 512-byte sectors (48-bit: low 32 + high 16).
    sector_count: u64,
    flags: FileFlags,
    file_acl: u64,
    generation: u32,
    /// Raw `i_block` (60 bytes) — the inline extent-tree root.
    block: [u32; RAW_BLOCK_PTRS_LEN],
}

/// `eh_magic` of an `ext4_extent_header`.
const EXTENT_MAGIC: u16 = 0xF30A;

/// Maximum extents the 60-byte inline root can hold past its 12-byte header
/// (`(60 - 12) / 12`).
const EXTENT_MAX_INLINE: u16 = 4;

/// Builds the inline extent-tree root for a freshly created inode: a valid empty
/// `ext4_extent_header` (magic `0xF30A`, 0 entries, max 4, depth 0) followed by
/// zeros. New regular files and directories carry this so the extent reader sees
/// a well-formed (empty) tree from the first byte — unlike ext2, whose new
/// inodes start with zeroed indirect-block pointers.
pub(in super::super) fn empty_extent_root() -> [u32; RAW_BLOCK_PTRS_LEN] {
    let mut block = [0u32; RAW_BLOCK_PTRS_LEN];
    // Each `i_block` word packs two 16-bit fields, little-endian: word 0 is
    // `eh_magic | eh_entries(=0)`, word 1 is `eh_max(=4) | eh_depth(=0)`.
    block[0] = u32::from(EXTENT_MAGIC);
    block[1] = u32::from(EXTENT_MAX_INLINE);
    block[2] = 0; // eh_generation
    block
}

impl InodeDesc {
    /// Builds a fresh inode descriptor for a newly created file or directory.
    ///
    /// Size and `i_blocks` start at zero and timestamps are set to `now`. On
    /// an extent volume the inline `i_block` contains an empty extent root;
    /// on an ext2-format volume it is an all-zero (fully sparse) indirect
    /// pointer array without the `EXTENTS` flag, so the volume stays mountable
    /// by a plain ext2 driver.
    #[expect(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        type_: InodeType,
        perm: FilePerm,
        uid: u32,
        gid: u32,
        link_count: u16,
        generation: u32,
        now: Duration,
        extent_based: bool,
    ) -> Self {
        let (flags, block) = if extent_based {
            (FileFlags::EXTENTS, empty_extent_root())
        } else {
            (FileFlags::empty(), [0u32; RAW_BLOCK_PTRS_LEN])
        };
        Self {
            type_,
            perm,
            uid,
            gid,
            size: 0,
            atime: now,
            ctime: now,
            mtime: now,
            crtime: now,
            dtime: Duration::ZERO,
            link_count,
            sector_count: 0,
            flags,
            file_acl: 0,
            generation,
            block,
        }
    }

    pub(in super::super) const fn type_(&self) -> InodeType {
        self.type_
    }

    /// Returns the extended-attribute block number (`i_file_acl`), decoded
    /// from the on-disk inode; 0 means the inode owns no EA block.
    pub(in super::super) const fn file_acl(&self) -> u64 {
        self.file_acl
    }

    pub(in super::super) const fn size(&self) -> u64 {
        self.size
    }

    pub(in super::super) const fn link_count(&self) -> u16 {
        self.link_count
    }

    pub(in super::super) const fn flags(&self) -> FileFlags {
        self.flags
    }

    pub(in super::super) const fn sector_count(&self) -> u64 {
        self.sector_count
    }

    /// Sets the logical file size (in bytes). Mutates through `Dirty`.
    pub(in super::super) fn set_size(&mut self, size: u64) {
        self.size = size;
    }

    /// Overwrites the link count outright. Mutates through `Dirty`.
    pub(in super::super) fn set_link_count(&mut self, count: u16) {
        self.link_count = count;
    }

    /// Adds `delta` to the link count. Mutates through `Dirty`.
    pub(in super::super) fn inc_link_count(&mut self, delta: u16) {
        debug_assert!(self.link_count <= u16::MAX - delta);
        self.link_count += delta;
    }

    /// Subtracts `delta` from the link count (saturating). Mutates through
    /// `Dirty`; used by the unlink/rmdir path to drop a name's reference.
    pub(in super::super) fn dec_link_count(&mut self, delta: u16) {
        debug_assert!(self.link_count >= delta);
        self.link_count = self.link_count.saturating_sub(delta);
    }

    /// Sets the deletion time (`i_dtime`). Mutates through `Dirty`; stamped when
    /// a fully unlinked inode is reclaimed.
    pub(in super::super) fn set_dtime(&mut self, time: Duration) {
        self.dtime = time;
    }

    /// Clears the given inode flags. Mutates through `Dirty`; used to drop the
    /// `EXTENTS` flag when an inode switches to inline (fast-symlink) storage.
    pub(in super::super) fn remove_flags(&mut self, flags: FileFlags) {
        self.flags.remove(flags);
    }

    /// Sets the given inode flags. Mutates through `Dirty`; used to restore the
    /// `EXTENTS` flag when a symlink switches from inline storage back to an
    /// extent-mapped data block.
    pub(in super::super) fn add_flags(&mut self, flags: FileFlags) {
        self.flags.insert(flags);
    }

    /// Overwrites the raw `i_block` words. Mutates through `Dirty`; used to store
    /// a fast-symlink target inline so writeback persists it (a fast symlink has
    /// no block manager to snapshot the `i_block` from).
    pub(in super::super) fn set_raw_block(&mut self, block: [u32; RAW_BLOCK_PTRS_LEN]) {
        self.block = block;
    }

    /// Sets the last-modification time. Mutates through `Dirty`.
    pub(in super::super) fn set_mtime(&mut self, time: Duration) {
        self.mtime = time;
    }

    /// Sets the last-metadata-change time. Mutates through `Dirty`.
    pub(in super::super) fn set_ctime(&mut self, time: Duration) {
        self.ctime = time;
    }

    /// Sets the extended-attribute block number (`i_file_acl`). Mutates through
    /// `Dirty`; 0 means the inode owns no EA block.
    pub(in super::super) fn set_file_acl(&mut self, file_acl: u64) {
        self.file_acl = file_acl;
    }

    /// Sets the `i_blocks` accounting (512-byte sectors). Mutates through
    /// `Dirty`; used to mirror the block manager's authoritative count.
    pub(in super::super) fn set_sector_count(&mut self, sectors: u64) {
        self.sector_count = sectors;
    }

    /// Sets the permission bits (chmod). Mutates through `Dirty`.
    pub(in super::super) fn set_perm(&mut self, perm: FilePerm) {
        self.perm = perm;
    }

    /// Sets the owning user id (chown). Mutates through `Dirty`.
    pub(in super::super) fn set_uid(&mut self, uid: u32) {
        self.uid = uid;
    }

    /// Sets the owning group id (chgrp). Mutates through `Dirty`.
    pub(in super::super) fn set_gid(&mut self, gid: u32) {
        self.gid = gid;
    }

    /// Sets the last-access time. Mutates through `Dirty`.
    pub(in super::super) fn set_atime(&mut self, time: Duration) {
        self.atime = time;
    }

    pub(in super::super) const fn perm(&self) -> FilePerm {
        self.perm
    }

    pub(in super::super) const fn uid(&self) -> u32 {
        self.uid
    }

    pub(in super::super) const fn gid(&self) -> u32 {
        self.gid
    }

    pub(in super::super) const fn atime(&self) -> Duration {
        self.atime
    }

    pub(in super::super) const fn mtime(&self) -> Duration {
        self.mtime
    }

    pub(in super::super) const fn ctime(&self) -> Duration {
        self.ctime
    }

    pub(in super::super) const fn crtime(&self) -> Duration {
        self.crtime
    }

    /// Returns the deletion time (`i_dtime`).
    pub(in super::super) const fn dtime(&self) -> Duration {
        self.dtime
    }

    /// Returns the inode generation (`i_generation`).
    pub(in super::super) const fn generation(&self) -> u32 {
        self.generation
    }

    /// Returns the raw `i_block` bytes holding the extent-tree root.
    pub(in super::super) const fn raw_block(&self) -> &[u32; RAW_BLOCK_PTRS_LEN] {
        &self.block
    }

    /// Returns whether this inode's data is mapped by an extent tree.
    pub(in super::super) fn is_extent_based(&self) -> bool {
        self.flags.contains(FileFlags::EXTENTS)
    }
}

/// Decodes an ext4 timestamp from its seconds field and the `*_extra` field.
///
/// The extra field packs a 2-bit epoch in its low bits and nanoseconds in the
/// upper bits.
fn decode_time(secs: u32, extra: u32) -> Duration {
    let epoch = (extra & 0x3) as u64;
    let nsec = extra >> 2;
    Duration::new((secs as u64) | (epoch << 32), nsec)
}

impl TryFrom<&RawInode> for InodeDesc {
    type Error = Error;

    fn try_from(raw: &RawInode) -> Result<Self> {
        if raw.link_count == 0 {
            return_errno_with_message!(Errno::ESTALE, "inode is not in use");
        }

        let type_ = InodeType::from_raw_mode(raw.mode)
            .map_err(|_| Error::with_message(Errno::EUCLEAN, "invalid inode mode"))?;
        let perm = FilePerm::from_bits_truncate(raw.mode);

        let uid = u32::from(raw.uid) | (u32::from(raw.uid_high) << 16);
        let gid = u32::from(raw.gid) | (u32::from(raw.gid_high) << 16);

        let mut size = raw.size_lo as u64;
        if type_ == InodeType::File {
            size |= (raw.size_high as u64) << 32;
        }
        if type_ == InodeType::SymLink && size >= BLOCK_SIZE as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "symlink size too large");
        }
        if size > i64::MAX as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "inode size too large");
        }

        let flags = FileFlags::from_bits_truncate(raw.flags);
        let sector_count = (raw.sector_count as u64) | ((raw.blocks_high as u64) << 32);
        let file_acl = (raw.file_acl_lo as u64) | ((raw.file_acl_high as u64) << 32);

        Ok(Self {
            type_,
            perm,
            uid,
            gid,
            size,
            atime: decode_time(raw.atime, raw.atime_extra),
            ctime: decode_time(raw.ctime, raw.ctime_extra),
            mtime: decode_time(raw.mtime, raw.mtime_extra),
            crtime: decode_time(raw.crtime, raw.crtime_extra),
            dtime: Duration::from_secs(raw.dtime as u64),
            link_count: raw.link_count,
            sector_count,
            flags,
            file_acl,
            generation: raw.generation,
            block: raw.block,
        })
    }
}

const_assert!(size_of::<RawInode>() == 256);

/// The on-disk ext4 inode (256 bytes for the default `s_inode_size`).
///
/// The first 128 bytes match ext2's layout; the trailing fields are ext4's
/// extra-size region (nanosecond timestamps, creation time) followed by space
/// reserved for inline extended attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in super::super) struct RawInode {
    pub mode: u16,
    pub uid: u16,
    pub size_lo: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub link_count: u16,
    /// `i_blocks` low 32 bits (512-byte sectors).
    pub sector_count: u32,
    pub flags: u32,
    pub osd1: u32,
    /// `i_block`: 60 bytes holding the inline extent-tree root.
    pub block: [u32; RAW_BLOCK_PTRS_LEN],
    pub generation: u32,
    pub file_acl_lo: u32,
    pub size_high: u32,
    pub obso_faddr: u32,
    // osd2 (Linux ext4 layout).
    pub blocks_high: u16,
    pub file_acl_high: u16,
    pub uid_high: u16,
    pub gid_high: u16,
    pub checksum_lo: u16,
    pub osd2_reserved: u16,
    // ext4 extra-size region (present when `s_inode_size` > 128).
    pub extra_isize: u16,
    pub checksum_hi: u16,
    pub ctime_extra: u32,
    pub mtime_extra: u32,
    pub atime_extra: u32,
    pub crtime: u32,
    pub crtime_extra: u32,
    pub version_hi: u32,
    pub projid: u32,
    /// Space reserved for inline extended attributes, which are unsupported.
    pub tail: InodeTail,
}

impl RawInode {
    /// Reads the on-disk inode slot at `offset`, honoring the volume's
    /// `inode_size`.
    ///
    /// Exactly `inode_size` bytes are read from the device; on a
    /// 128-byte-inode volume the missing tail of the returned 256-byte
    /// `RawInode` is zero-filled, so the extra fields (`extra_isize`,
    /// sub-second timestamps, `crtime`) decode to their absent defaults
    /// without any decoding branch.
    pub(in super::super) fn read_from_slot(
        device: &Arc<dyn BlockDevice>,
        offset: usize,
        inode_size: usize,
    ) -> Result<Self> {
        let mut bytes = [0u8; size_of::<Self>()];
        let n = inode_size.min(size_of::<Self>());
        device
            .read_bytes(offset, &mut bytes[..n])
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read inode slot"))?;
        Ok(Self::from_bytes(&bytes))
    }

    /// Writes this inode back to its slot at `offset`, honoring the volume's
    /// `inode_size`.
    ///
    /// Only the first `inode_size` bytes are written, so on a 128-byte-inode
    /// volume the bytes past the slot (the next inode's storage) are never
    /// touched.
    pub(in super::super) fn write_to_slot(
        &self,
        device: &Arc<dyn BlockDevice>,
        offset: usize,
        inode_size: usize,
    ) -> Result<()> {
        let n = inode_size.min(size_of::<Self>());
        device
            .write_bytes(offset, &self.as_bytes()[..n])
            .map_err(|_| Error::with_message(Errno::EIO, "failed to write inode slot"))?;
        Ok(())
    }

    /// Updates the modeled fields while preserving unsupported on-disk fields.
    pub(in super::super) fn update_from_desc(
        &mut self,
        desc: &InodeDesc,
        root: &[u32; RAW_BLOCK_PTRS_LEN],
    ) -> Result<()> {
        let (size_lo, size_hi) = split_u64(desc.size());
        self.size_lo = size_lo;
        if desc.type_() == InodeType::File {
            self.size_high = size_hi;
        }

        let (sectors_lo, sectors_hi) = split_u48(desc.sector_count())?;
        self.sector_count = sectors_lo;
        self.blocks_high = sectors_hi;

        let (file_acl_lo, file_acl_high) = split_u48(desc.file_acl())?;
        self.file_acl_lo = file_acl_lo;
        self.file_acl_high = file_acl_high;

        (self.mtime, self.mtime_extra) = encode_time(desc.mtime());
        (self.ctime, self.ctime_extra) = encode_time(desc.ctime());
        (self.atime, self.atime_extra) = encode_time(desc.atime());

        self.mode = (self.mode & 0xF000) | (desc.perm().bits() & 0o7777);
        (self.uid, self.uid_high) = split_u32(desc.uid());
        (self.gid, self.gid_high) = split_u32(desc.gid());
        self.block = *root;
        self.flags = desc.flags().bits();
        self.link_count = desc.link_count();
        self.dtime = u32::try_from(desc.dtime().as_secs())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "deletion time is too large"))?;
        Ok(())
    }

    /// Encodes every field of a newly allocated inode slot.
    pub(in super::super) fn from_desc(desc: &InodeDesc) -> Result<Self> {
        let mut raw = Self {
            mode: (desc.type_() as u16) | (desc.perm().bits() & 0o7777),
            generation: desc.generation(),
            extra_isize: 32,
            ..Default::default()
        };
        raw.update_from_desc(desc, desc.raw_block())?;
        (raw.crtime, raw.crtime_extra) = encode_time(desc.crtime());
        Ok(raw)
    }
}

/// Reads the special-file device encoding stored in `i_block`.
///
/// The layout is shared by ext2 and ext4: the old 8-bit-major/minor encoding
/// lives in word 0, the extended encoding in word 1.
pub(in super::super) fn read_device_id(block: &[u32; RAW_BLOCK_PTRS_LEN]) -> u64 {
    let (major, minor) = if block[0] != 0 {
        let old_encoded_device = block[0];
        // Old_decode_dev: (major << 8) | minor with 8-bit major/minor.
        (
            ((old_encoded_device >> 8) & 0xFF),
            (old_encoded_device & 0xFF),
        )
    } else {
        let encoded_device = block[1];
        // Decode the extended major/minor bit layout.
        (
            ((encoded_device & 0xFFF00) >> 8),
            ((encoded_device & 0xFF) | ((encoded_device >> 12) & 0xFFF00)),
        )
    };

    encode_device_numbers(major, minor)
}

/// Writes a device ID into the special-file `i_block` layout.
pub(in super::super) fn write_device_id(block: &mut [u32; RAW_BLOCK_PTRS_LEN], device_id: u64) {
    let (major, minor) = decode_device_numbers(device_id);

    // Old_valid_dev: MAJOR/MINOR must both fit in 8 bits.
    if major < 256 && minor < 256 {
        block[0] = (major << 8) | minor;
        block[1] = 0;
    } else {
        block[0] = 0;
        block[1] = (minor & 0xFF) | (major << 8) | ((minor & !0xFF) << 12);
        block[2] = 0;
    }
}

fn split_u64(value: u64) -> (u32, u32) {
    let low = u32::try_from(value & u64::from(u32::MAX)).expect("masked low half fits u32");
    let high = u32::try_from(value >> 32).expect("shifted high half fits u32");
    (low, high)
}

fn split_u48(value: u64) -> Result<(u32, u16)> {
    if value >> 48 != 0 {
        return Err(Error::with_message(
            Errno::EOVERFLOW,
            "value exceeds the 48-bit on-disk field",
        ));
    }
    let (low, high) = split_u64(value);
    Ok((
        low,
        u16::try_from(high).expect("48-bit value has a 16-bit high half"),
    ))
}

fn split_u32(value: u32) -> (u16, u16) {
    let low = u16::try_from(value & u32::from(u16::MAX)).expect("masked low half fits u16");
    let high = u16::try_from(value >> 16).expect("shifted high half fits u16");
    (low, high)
}

fn encode_time(time: Duration) -> (u32, u32) {
    let secs = time.as_secs();
    let epoch = u32::try_from((secs >> 32) & 0x3).expect("masked epoch fits u32");
    let secs_lo = u32::try_from(secs & u64::from(u32::MAX)).expect("masked seconds fit u32");
    (secs_lo, epoch | (time.subsec_nanos() << 2))
}

/// Padding from the end of the ext4 inode fields (offset 160) to the 256-byte
/// on-disk inode size; in ext4 this holds inline extended attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct InodeTail(pub(in super::super) [u8; 96]);

impl Default for InodeTail {
    fn default() -> Self {
        Self([0u8; 96])
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{super::EXTENTS_FL, *};

    /// Builds a raw root-directory inode: one data block via an (unparsed here)
    /// extent root, link count 2, `EXT4_EXTENTS_FL` set.
    fn raw_root_dir() -> RawInode {
        RawInode {
            mode: 0o040755, // S_IFDIR | 0755
            size_lo: BLOCK_SIZE as u32,
            link_count: 2,
            sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
            flags: EXTENTS_FL,
            extra_isize: 32,
            ..Default::default()
        }
    }

    #[ktest]
    fn reject_unused_inode() {
        let mut raw = raw_root_dir();
        raw.link_count = 0;
        assert!(InodeDesc::try_from(&raw).is_err());
    }

    #[ktest]
    fn reject_sector_count_exceeding_disk_field() {
        let raw = raw_root_dir();
        let mut desc = InodeDesc::try_from(&raw).unwrap();
        desc.set_sector_count(1 << 48);
        assert!(RawInode::from_desc(&desc).is_err());
    }
}
