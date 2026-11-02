// SPDX-License-Identifier: MPL-2.0

//! F2FS inode representation, read paths, and write paths.
//!
//! An F2FS inode is not only a metadata record: small files and directories
//! may keep their data or their directory entries inside the inode's address
//! array ("inline data" and "inline dentry"). Larger files use direct node
//! blocks, indirect node blocks, and a double-indirect node, all of which are
//! addressed through the NAT area.
//!
//! # Address layout
//!
//! File block `i` is resolved as follows, where `N` is the number of address
//! pointers available in the inode itself:
//!
//! ```text
//! i < N                      -> i_addr[i]
//! i < N + 1018               -> direct node 1
//! i < N + 2 * 1018           -> direct node 2
//! i < N + 3 * 1018           -> indirect node 1
//! i < N + 4 * 1018           -> indirect node 2
//! otherwise                  -> double-indirect node
//! ```
//!
//! New blocks are allocated in place from the SIT and recorded in the SSA.
//! Inline directories are converted to block-backed directories on their
//! first mutation.

use core::time::Duration;

use device_id::DeviceId;

use super::{
    alloc::{BlockKind, WriteState},
    fs::{Block, F2fs, zeroed_block},
    ondisk::{
        ADDRS_PER_BLOCK, DEF_ADDRS_PER_INODE, DEF_INLINE_RESERVED_SIZE, DEFAULT_INLINE_XATTR_ADDRS,
        DENTRY_BITMAP_SIZE, DENTRY_RESERVED_SIZE, DENTRY_SIZE, F2FS_BLKSIZE, F2FS_DATA_EXIST,
        F2FS_EXTRA_ATTR, F2FS_FEATURE_FLEXIBLE_INLINE_XATTR, F2FS_FEATURE_INODE_CRTIME,
        F2FS_INLINE_DATA, F2FS_INLINE_DENTRY, F2FS_INLINE_DOTS, F2FS_INLINE_XATTR,
        INODE_ADDR_OFFSET, INODE_NID_OFFSET, NAME_MAX, NIDS_PER_INODE, NODE_FOOTER_OFFSET,
        NR_DENTRY_IN_BLOCK, SLOT_LEN, dentry_hash, inode_type_from_dentry, inode_type_from_mode,
        read_u16, read_u32, read_u64, test_bit_le,
    },
    prelude::*,
};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        utils::DirentVisitor,
        vfs::inode::{Extension, Metadata, SymbolicLink},
    },
    process::{Gid, Uid},
    time::clocks::RealTimeCoarseClock,
};

/// Upper bound on the directory size this driver is willing to scan.
const MAX_DIRECTORY_SIZE: u64 = 1 << 30;

/// A decoded F2FS timestamp.
#[derive(Clone, Copy, Debug, Default)]
struct Timestamp {
    secs: u64,
    nsec: u32,
}

impl Timestamp {
    fn now() -> Self {
        let duration = RealTimeCoarseClock::get().read_time();
        Self {
            secs: duration.as_secs(),
            nsec: duration.subsec_nanos(),
        }
    }

    fn to_duration(self) -> Duration {
        Duration::from_secs(self.secs)
            .saturating_add(Duration::from_nanos(self.nsec.min(999_999_999) as u64))
    }
}

/// Copies a byte slice into a VFS output buffer.
fn copy_to_writer(writer: &mut VmWriter, data: &[u8]) -> Result<()> {
    let mut reader = VmReader::from(data);
    let written = writer
        .write_fallible(&mut reader)
        .map_err(|_| Error::with_message(Errno::EFAULT, "failed to write F2FS file data"))?;
    if written != data.len() {
        return_errno_with_message!(Errno::EFAULT, "short write to the output buffer");
    }
    Ok(())
}

fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Returns the largest file size representable by the address tree.
fn max_file_size(addr_count: usize) -> u64 {
    let per_block = ADDRS_PER_BLOCK as u64;
    let blocks = addr_count as u64
        + 2 * per_block
        + 2 * per_block * per_block
        + per_block * per_block * per_block;
    blocks.saturating_mul(F2FS_BLKSIZE as u64)
}

/// Returns the POSIX type bits of an inode type.
fn type_mode(type_: InodeType) -> u16 {
    match type_ {
        InodeType::NamedPipe => 0o010000,
        InodeType::CharDevice => 0o020000,
        InodeType::Dir => 0o040000,
        InodeType::BlockDevice => 0o060000,
        InodeType::File => 0o100000,
        InodeType::SymLink => 0o120000,
        InodeType::Socket => 0o140000,
        InodeType::Unknown => 0,
    }
}

/// Returns the on-disk directory entry type of an inode type.
fn dentry_file_type(type_: InodeType) -> u8 {
    match type_ {
        InodeType::File => 1,
        InodeType::Dir => 2,
        InodeType::CharDevice => 3,
        InodeType::BlockDevice => 4,
        InodeType::NamedPipe => 5,
        InodeType::Socket => 6,
        InodeType::SymLink => 7,
        InodeType::Unknown => 0,
    }
}

/// One directory entry.
pub(super) struct DirEntry {
    pub name: String,
    pub ino: u32,
    pub type_: InodeType,
}

/// Mutable state of an inode.
struct InodeState {
    ino: u32,
    node_addr: u32,
    mode: u16,
    type_: InodeType,
    uid: u32,
    gid: u32,
    links: u32,
    size: u64,
    blocks: u64,
    atime: Timestamp,
    ctime: Timestamp,
    mtime: Timestamp,
    crtime: Option<Timestamp>,
    parent_ino: u32,
    inline_flags: u8,
    extra_isize_addrs: usize,
    inline_xattr_addrs: usize,
    device_id: u32,
    cp_ver: u64,
    node: Block,
}

impl InodeState {
    fn addr_count(&self) -> usize {
        DEF_ADDRS_PER_INODE - self.extra_isize_addrs - self.inline_xattr_addrs
    }

    fn inline_base(&self) -> usize {
        INODE_ADDR_OFFSET + self.extra_isize_addrs * 4 + DEF_INLINE_RESERVED_SIZE * 4
    }

    fn inline_capacity(&self) -> usize {
        (self.addr_count() - DEF_INLINE_RESERVED_SIZE) * 4
    }

    fn has_inline_data(&self) -> bool {
        self.size > 0
            && self.inline_flags & F2FS_INLINE_DATA != 0
            && self.inline_flags & F2FS_DATA_EXIST != 0
    }

    fn inode_addr(&self, index: usize) -> u32 {
        read_u32(
            self.node.as_slice(),
            INODE_ADDR_OFFSET + (self.extra_isize_addrs + index) * 4,
        )
    }

    fn set_inode_addr(&mut self, index: usize, value: u32) {
        write_u32(
            self.node.as_mut_slice(),
            INODE_ADDR_OFFSET + (self.extra_isize_addrs + index) * 4,
            value,
        );
    }

    fn nid(&self, index: usize) -> u32 {
        read_u32(self.node.as_slice(), INODE_NID_OFFSET + index * 4)
    }

    fn set_nid(&mut self, index: usize, value: u32) {
        write_u32(
            self.node.as_mut_slice(),
            INODE_NID_OFFSET + index * 4,
            value,
        );
    }

    fn sync_fields_to_node(&mut self) {
        write_u16(self.node.as_mut_slice(), 0, self.mode);
        self.node[3] = self.inline_flags;
        write_u32(self.node.as_mut_slice(), 4, self.uid);
        write_u32(self.node.as_mut_slice(), 8, self.gid);
        write_u32(self.node.as_mut_slice(), 12, self.links);
        write_u64(self.node.as_mut_slice(), 16, self.size);
        write_u64(self.node.as_mut_slice(), 24, self.blocks);
        write_u64(self.node.as_mut_slice(), 32, self.atime.secs);
        write_u64(self.node.as_mut_slice(), 40, self.ctime.secs);
        write_u64(self.node.as_mut_slice(), 48, self.mtime.secs);
        write_u32(self.node.as_mut_slice(), 56, self.atime.nsec);
        write_u32(self.node.as_mut_slice(), 60, self.ctime.nsec);
        write_u32(self.node.as_mut_slice(), 64, self.mtime.nsec);
        write_u32(self.node.as_mut_slice(), 84, self.parent_ino);
    }

    fn touch(&mut self) {
        let now = Timestamp::now();
        self.mtime = now;
        self.ctime = now;
    }
}

/// A read/write F2FS inode.
pub(crate) struct Inode {
    fs: Weak<F2fs>,
    ino: u32,
    state: RwMutex<InodeState>,
    extension: Extension,
}

impl Inode {
    /// Loads and parses the inode numbered `ino`.
    pub(super) fn load(fs: &Arc<F2fs>, ino: u32) -> Result<Arc<Self>> {
        if let Some(inode) = fs.cached_inode(ino) {
            return Ok(inode);
        }

        let entry = fs.lookup_nat(ino)?;
        let node = fs.read_node(ino)?;

        let mode = read_u16(node.as_slice(), 0);
        let inline_flags = node[3];
        let type_ = inode_type_from_mode(mode);

        let extra_isize_bytes = if inline_flags & F2FS_EXTRA_ATTR != 0 {
            read_u16(node.as_slice(), INODE_ADDR_OFFSET) as usize
        } else {
            0
        };
        if extra_isize_bytes % 4 != 0 || extra_isize_bytes / 4 >= DEF_ADDRS_PER_INODE {
            return_errno_with_message!(Errno::EIO, "invalid F2FS extra inode size");
        }
        let extra_isize_addrs = extra_isize_bytes / 4;

        let inline_xattr_addrs =
            if fs.super_block().feature & F2FS_FEATURE_FLEXIBLE_INLINE_XATTR != 0 {
                if inline_flags & F2FS_EXTRA_ATTR != 0 {
                    read_u16(node.as_slice(), INODE_ADDR_OFFSET + 2) as usize
                } else {
                    0
                }
            } else if inline_flags & (F2FS_INLINE_XATTR | F2FS_INLINE_DENTRY) != 0 {
                DEFAULT_INLINE_XATTR_ADDRS
            } else {
                0
            };
        if extra_isize_addrs + inline_xattr_addrs >= DEF_ADDRS_PER_INODE {
            return_errno_with_message!(Errno::EIO, "invalid F2FS inline xattr size");
        }

        let device_id = if matches!(
            type_,
            InodeType::CharDevice
                | InodeType::BlockDevice
                | InodeType::NamedPipe
                | InodeType::Socket
        ) {
            let addr_base = INODE_ADDR_OFFSET + extra_isize_addrs * 4;
            let old_dev = read_u32(node.as_slice(), addr_base);
            if old_dev != 0 {
                old_dev
            } else {
                read_u32(node.as_slice(), addr_base + 4)
            }
        } else {
            0
        };

        let size = read_u64(node.as_slice(), 16);
        let max_size = max_file_size(DEF_ADDRS_PER_INODE - extra_isize_addrs - inline_xattr_addrs);
        if size > max_size {
            return_errno_with_message!(Errno::EIO, "the F2FS inode size is out of range");
        }
        if type_ == InodeType::Dir {
            if inline_flags & F2FS_INLINE_DENTRY != 0 {
                let capacity = (DEF_ADDRS_PER_INODE
                    - extra_isize_addrs
                    - inline_xattr_addrs
                    - DEF_INLINE_RESERVED_SIZE)
                    * 4;
                if size == 0 || size > capacity as u64 {
                    return_errno_with_message!(Errno::EIO, "invalid inline directory size");
                }
            } else if !size.is_multiple_of(F2FS_BLKSIZE as u64) || size > MAX_DIRECTORY_SIZE {
                return_errno_with_message!(Errno::EIO, "the F2FS directory size is invalid");
            }
        }

        let crtime = if fs.super_block().feature & F2FS_FEATURE_INODE_CRTIME != 0
            && inline_flags & F2FS_EXTRA_ATTR != 0
            && extra_isize_bytes >= 24
        {
            Some(Timestamp {
                secs: read_u64(node.as_slice(), 372),
                nsec: read_u32(node.as_slice(), 380),
            })
        } else {
            None
        };

        let state = InodeState {
            ino,
            node_addr: entry.block_addr,
            mode,
            type_,
            uid: read_u32(node.as_slice(), 4),
            gid: read_u32(node.as_slice(), 8),
            links: read_u32(node.as_slice(), 12),
            size,
            blocks: read_u64(node.as_slice(), 24),
            atime: Timestamp {
                secs: read_u64(node.as_slice(), 32),
                nsec: read_u32(node.as_slice(), 56),
            },
            ctime: Timestamp {
                secs: read_u64(node.as_slice(), 40),
                nsec: read_u32(node.as_slice(), 60),
            },
            mtime: Timestamp {
                secs: read_u64(node.as_slice(), 48),
                nsec: read_u32(node.as_slice(), 64),
            },
            crtime,
            parent_ino: read_u32(node.as_slice(), 84),
            inline_flags,
            extra_isize_addrs,
            inline_xattr_addrs,
            device_id,
            cp_ver: fs.checkpoint().version,
            node,
        };

        let inode = Arc::new(Self {
            fs: fs.self_ref(),
            ino,
            state: RwMutex::new(state),
            extension: Extension::new(),
        });
        if let Some(existing) = fs.cache_inode(ino, Arc::downgrade(&inode)) {
            return Ok(existing);
        }

        Ok(inode)
    }

    /// Creates and persists a new inode, returning its number.
    fn build_new_inode(
        fs: &Arc<F2fs>,
        state: &mut WriteState,
        type_: InodeType,
        mode: InodeMode,
        parent_ino: u32,
    ) -> Result<(u32, u32, Option<u32>)> {
        let ino = fs.find_free_nid(state)?;
        let (_, node_addr) = fs.alloc_node(state, ino, Some(ino))?;
        state.valid_inode_count = state.valid_inode_count.saturating_add(1);

        let mut node = zeroed_block();
        write_u16(node.as_mut_slice(), 0, type_mode(type_) | mode.bits());
        write_u32(node.as_mut_slice(), 4, 0);
        write_u32(node.as_mut_slice(), 8, 0);
        write_u32(
            node.as_mut_slice(),
            12,
            if type_ == InodeType::Dir { 2 } else { 1 },
        );
        write_u64(node.as_mut_slice(), 24, 1);
        let now = Timestamp::now();
        write_u64(node.as_mut_slice(), 32, now.secs);
        write_u64(node.as_mut_slice(), 40, now.secs);
        write_u64(node.as_mut_slice(), 48, now.secs);
        write_u32(node.as_mut_slice(), 56, now.nsec);
        write_u32(node.as_mut_slice(), 60, now.nsec);
        write_u32(node.as_mut_slice(), 64, now.nsec);
        if type_ == InodeType::Dir {
            write_u32(node.as_mut_slice(), 72, 1);
        }
        write_u32(node.as_mut_slice(), 84, parent_ino);

        // Node footer.
        write_u32(node.as_mut_slice(), NODE_FOOTER_OFFSET, ino);
        write_u32(node.as_mut_slice(), NODE_FOOTER_OFFSET + 4, ino);
        write_u64(
            node.as_mut_slice(),
            NODE_FOOTER_OFFSET + 12,
            fs.checkpoint().version,
        );

        let mut data_block = None;
        if type_ == InodeType::Dir {
            let data_addr = fs.alloc_block(state, BlockKind::Data)?;
            data_block = Some(data_addr);
            write_u32(node.as_mut_slice(), INODE_ADDR_OFFSET, data_addr);
            write_u64(node.as_mut_slice(), 16, F2FS_BLKSIZE as u64);
            write_u64(node.as_mut_slice(), 24, 2);
            fs.write_summary(data_addr, ino, 0, BlockKind::Data)?;

            let mut block = zeroed_block();
            write_dentry(block.as_mut_slice(), 0, ".", ino, 2)?;
            write_dentry(block.as_mut_slice(), 1, "..", parent_ino, 2)?;
            fs.write_data_block(data_addr, &block)?;
        }

        fs.write_data_block(node_addr, &node)?;
        Ok((ino, node_addr, data_block))
    }

    /// Rolls back a freshly created inode when linking it into the parent
    /// directory fails.
    fn discard_new_inode(
        fs: &Arc<F2fs>,
        alloc: &mut WriteState,
        node_addr: u32,
        data_addr: Option<u32>,
        ino: u32,
    ) -> Result<()> {
        if let Some(data_addr) = data_addr {
            fs.free_block(alloc, data_addr)?;
        }
        fs.free_block(alloc, node_addr)?;
        fs.free_nid(ino)?;
        alloc.valid_inode_count = alloc.valid_inode_count.saturating_sub(1);
        Ok(())
    }

    /// Returns the filesystem that owns this inode.
    pub(super) fn fs(&self) -> Result<Arc<F2fs>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "the F2FS instance is gone"))
    }

    pub(super) fn ino(&self) -> u32 {
        self.ino
    }

    pub(super) fn file_size(&self) -> usize {
        self.state.read().size as usize
    }

    pub(super) fn inode_type(&self) -> InodeType {
        self.state.read().type_
    }

    pub(super) fn perm(&self) -> InodeMode {
        InodeMode::from_bits_truncate(self.state.read().mode & 0o7777)
    }

    pub(super) fn uid(&self) -> Uid {
        Uid::new(self.state.read().uid)
    }

    pub(super) fn gid(&self) -> Gid {
        Gid::new(self.state.read().gid)
    }

    pub(super) fn atime(&self) -> Duration {
        self.state.read().atime.to_duration()
    }

    pub(super) fn mtime(&self) -> Duration {
        self.state.read().mtime.to_duration()
    }

    pub(super) fn ctime(&self) -> Duration {
        self.state.read().ctime.to_duration()
    }

    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }

    /// Persists the inode node.
    fn write_back(fs: &Arc<F2fs>, state: &mut InodeState) -> Result<()> {
        state.sync_fields_to_node();
        fs.write_data_block(state.node_addr, &state.node)
    }

    /// Reads file data starting at `offset` into `writer`.
    pub(super) fn read_data(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let fs = self.fs()?;
        let state = self.state.read();
        let size = state.size as usize;
        if offset >= size || writer.avail() == 0 {
            return Ok(0);
        }
        let read_len = (size - offset).min(writer.avail());

        if state.has_inline_data() {
            let capacity = state.inline_capacity();
            if offset + read_len > capacity {
                return_errno_with_message!(Errno::EIO, "inline F2FS data is out of range");
            }
            let base = state.inline_base();
            copy_to_writer(writer, &state.node[base + offset..base + offset + read_len])?;
            return Ok(read_len);
        }

        let zeros = [0u8; F2FS_BLKSIZE];
        let mut copied = 0;
        while copied < read_len {
            let file_offset = offset + copied;
            let index = file_offset / F2FS_BLKSIZE;
            let block_offset = file_offset % F2FS_BLKSIZE;
            let len = (F2FS_BLKSIZE - block_offset).min(read_len - copied);
            let addr = get_data_addr(&fs, &state, index)?;

            if addr == 0 || addr == u32::MAX {
                copy_to_writer(writer, &zeros[block_offset..block_offset + len])?;
            } else {
                let block = fs.read_data_block(addr)?;
                copy_to_writer(writer, &block[block_offset..block_offset + len])?;
            }
            copied += len;
        }

        Ok(copied)
    }

    /// Writes `data` at `offset`, extending the file when necessary.
    pub(super) fn write_bytes(&self, offset: usize, data: &[u8]) -> Result<usize> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }

        let mut alloc = fs.write_state();
        let mut state = self.state.write();
        if state.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
        }
        if state.has_inline_data() {
            materialize_inline_data(&fs, &mut alloc, &mut state)?;
        }

        let end = offset
            .checked_add(data.len())
            .ok_or_else(|| Error::with_message(Errno::EFBIG, "F2FS write offset overflow"))?;
        if end as u64 > max_file_size(state.addr_count()) {
            return_errno_with_message!(Errno::EFBIG, "the F2FS file is too large");
        }

        let mut done = 0;
        while done < data.len() {
            let file_offset = offset
                .checked_add(done)
                .ok_or_else(|| Error::with_message(Errno::EFBIG, "F2FS write offset overflow"))?;
            let index = file_offset / F2FS_BLKSIZE;
            let block_offset = file_offset % F2FS_BLKSIZE;
            let len = (F2FS_BLKSIZE - block_offset).min(data.len() - done);

            let addr = get_data_addr(&fs, &state, index)?;
            let (block_addr, mut block) = if addr == 0 || addr == u32::MAX {
                let new_addr = fs.alloc_block(&mut alloc, BlockKind::Data)?;
                let (nid, ofs) = set_data_addr(&fs, &mut alloc, &mut state, index, new_addr)?;
                fs.write_summary(new_addr, nid, ofs, BlockKind::Data)?;
                state.blocks = state.blocks.saturating_add(1);
                (new_addr, zeroed_block())
            } else {
                (addr, fs.read_data_block(addr)?)
            };

            block[block_offset..block_offset + len].copy_from_slice(&data[done..done + len]);
            fs.write_data_block(block_addr, &block)?;
            done += len;
        }

        if end as u64 > state.size {
            state.size = end as u64;
        }
        state.touch();
        Self::write_back(&fs, &mut state)?;
        Ok(done)
    }

    /// Writes data read from a VFS input buffer.
    ///
    /// The input is consumed in bounded chunks so that a large `write` does
    /// not size a kernel allocation from the user-supplied length.
    pub(super) fn write_from_reader(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        const CHUNK: usize = 64 * 1024;

        let mut buf = vec![0u8; CHUNK];
        let mut written = 0;
        while reader.remain() > 0 {
            let want = reader.remain().min(CHUNK);
            let mut writer = VmWriter::from(&mut buf[..want]).to_fallible();
            let read = reader
                .read_fallible(&mut writer)
                .map_err(|_| Error::with_message(Errno::EFAULT, "failed to read the input"))?;
            if read == 0 {
                break;
            }
            let chunk_offset = offset
                .checked_add(written)
                .ok_or_else(|| Error::with_message(Errno::EFBIG, "F2FS write offset overflow"))?;
            written += self.write_bytes(chunk_offset, &buf[..read])?;
        }
        Ok(written)
    }

    /// Changes the size of the file, freeing blocks beyond the new end.
    pub(super) fn truncate_to(&self, new_size: usize) -> Result<()> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }

        let mut alloc = fs.write_state();
        let mut state = self.state.write();
        if state.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
        }

        if new_size as u64 > max_file_size(state.addr_count()) {
            return_errno_with_message!(Errno::EFBIG, "the F2FS file is too large");
        }
        if state.has_inline_data() {
            if new_size == 0 {
                state.inline_flags &= !(F2FS_INLINE_DATA | F2FS_DATA_EXIST);
            }
            state.size = new_size as u64;
            state.touch();
            return Self::write_back(&fs, &mut state);
        }

        let old_blocks = (state.size as usize).div_ceil(F2FS_BLKSIZE);
        let new_blocks = new_size.div_ceil(F2FS_BLKSIZE);
        if new_size < state.size as usize {
            for index in new_blocks..old_blocks {
                let addr = clear_data_addr(&fs, &mut state, index)?;
                if addr != 0 && addr != u32::MAX {
                    fs.free_block(&mut alloc, addr)?;
                    state.blocks = state.blocks.saturating_sub(1);
                }
            }
            if new_size == 0 {
                free_node_blocks(&fs, &mut alloc, &mut state)?;
            }
        }

        state.size = new_size as u64;
        state.touch();
        Self::write_back(&fs, &mut state)
    }

    /// Parses the directory entries of a dentry region.
    fn parse_dentry_region(
        buf: &[u8],
        base: usize,
        count: usize,
        bitmap_size: usize,
        reserved_size: usize,
        entries: &mut Vec<DirEntry>,
    ) -> Result<()> {
        let bitmap = &buf[base..base + bitmap_size];
        let entry_base = base + bitmap_size + reserved_size;
        let name_base = entry_base + count * DENTRY_SIZE;

        let mut slot = 0;
        while slot < count {
            if !test_bit_le(bitmap, slot) {
                slot += 1;
                continue;
            }

            let entry_offset = entry_base + slot * DENTRY_SIZE;
            let ino = read_u32(buf, entry_offset + 4);
            let name_len = read_u16(buf, entry_offset + 8) as usize;
            let file_type = buf[entry_offset + 10];
            let slots = name_len.div_ceil(SLOT_LEN).max(1);
            if name_len == 0 || slot + slots > count {
                slot += 1;
                continue;
            }

            let name_offset = name_base + slot * SLOT_LEN;
            let name = core::str::from_utf8(&buf[name_offset..name_offset + name_len])
                .map_err(|_| Error::with_message(Errno::EIO, "invalid F2FS directory entry name"))?
                .to_string();
            if ino != 0 {
                entries.push(DirEntry {
                    name,
                    ino,
                    type_: inode_type_from_dentry(file_type),
                });
            }

            slot += slots;
        }

        Ok(())
    }

    /// Collects all directory entries while the inode lock is held.
    fn entries_locked(fs: &Arc<F2fs>, state: &InodeState) -> Result<Vec<DirEntry>> {
        if state.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the inode is not a directory");
        }

        let mut entries = Vec::new();
        if state.inline_flags & F2FS_INLINE_DENTRY != 0 {
            if state.inline_flags & F2FS_INLINE_DOTS != 0 {
                entries.push(DirEntry {
                    name: ".".to_string(),
                    ino: state.ino,
                    type_: InodeType::Dir,
                });
                entries.push(DirEntry {
                    name: "..".to_string(),
                    ino: state.parent_ino,
                    type_: InodeType::Dir,
                });
            }

            let capacity = state.inline_capacity();
            let count = capacity * 8 / ((DENTRY_SIZE + SLOT_LEN) * 8 + 1);
            let bitmap_size = count.div_ceil(8);
            let reserved_size =
                capacity.saturating_sub((DENTRY_SIZE + SLOT_LEN) * count + bitmap_size);
            let base = state.inline_base();
            Self::parse_dentry_region(
                state.node.as_slice(),
                base,
                count,
                bitmap_size,
                reserved_size,
                &mut entries,
            )?;
        } else {
            let block_count = (state.size as usize).div_ceil(F2FS_BLKSIZE);
            for index in 0..block_count {
                let addr = get_data_addr(fs, state, index)?;
                if addr == 0 || addr == u32::MAX {
                    continue;
                }
                let block = fs.read_data_block(addr)?;
                Self::parse_dentry_region(
                    block.as_slice(),
                    0,
                    NR_DENTRY_IN_BLOCK,
                    DENTRY_BITMAP_SIZE,
                    DENTRY_RESERVED_SIZE,
                    &mut entries,
                )?;
            }
        }

        Ok(entries)
    }

    /// Collects all directory entries of this inode.
    pub(super) fn collect_dentries(&self) -> Result<Vec<DirEntry>> {
        let fs = self.fs()?;
        let state = self.state.read();
        Self::entries_locked(&fs, &state)
    }

    /// Looks up a child by name.
    pub(super) fn lookup(&self, name: &str) -> Result<Arc<Inode>> {
        let fs = self.fs()?;
        let entries = {
            let state = self.state.read();
            Self::entries_locked(&fs, &state)?
        };
        for entry in entries {
            if entry.name == name {
                return Inode::load(&fs, entry.ino);
            }
        }
        return_errno_with_message!(Errno::ENOENT, "the directory entry does not exist");
    }

    /// Iterates directory entries starting at the given entry index.
    pub(super) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let entries = self.collect_dentries()?;
        let mut visited = 0;
        for (index, entry) in entries.iter().enumerate().skip(offset) {
            if visitor
                .visit(&entry.name, entry.ino as u64, entry.type_, index + 1)
                .is_err()
            {
                break;
            }
            visited += 1;
        }
        Ok(visited)
    }

    /// Reads the target of a symbolic link.
    pub(super) fn read_link_target(&self) -> Result<SymbolicLink> {
        if self.inode_type() != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "the inode is not a symbolic link");
        }

        let len = self.file_size().min(F2FS_BLKSIZE);
        let mut buf = vec![0u8; len];
        {
            let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
            self.read_data(0, &mut writer)?;
        }
        let target = String::from_utf8(buf)
            .map_err(|_| Error::with_message(Errno::EIO, "the symlink target is not UTF-8"))?;
        Ok(SymbolicLink::Plain(target))
    }

    /// Builds the VFS metadata of this inode.
    pub(super) fn to_metadata(&self) -> Result<Metadata> {
        let fs = self.fs()?;
        let state = self.state.read();
        let self_dev_id = if matches!(state.type_, InodeType::CharDevice | InodeType::BlockDevice) {
            DeviceId::from_encoded_u64(state.device_id as u64)
        } else {
            None
        };

        Ok(Metadata {
            ino: self.ino as u64,
            size: state.size as usize,
            optimal_block_size: F2FS_BLKSIZE,
            nr_sectors_allocated: (state.blocks.saturating_sub(1) as usize)
                .saturating_mul(F2FS_BLKSIZE / 512),
            last_access_at: state.atime.to_duration(),
            last_modify_at: state.mtime.to_duration(),
            last_meta_change_at: state.ctime.to_duration(),
            type_: state.type_,
            mode: InodeMode::from_bits_truncate(state.mode & 0o7777),
            nr_hard_links: state.links as usize,
            uid: Uid::new(state.uid),
            gid: Gid::new(state.gid),
            container_dev_id: fs.container_device_id(),
            self_dev_id,
            birth_at: state.crtime.map(Timestamp::to_duration),
        })
    }

    /// Updates the permission bits.
    pub(super) fn set_perm(&self, mode: InodeMode) -> Result<()> {
        let fs = self.fs()?;
        let mut state = self.state.write();
        state.mode = (state.mode & 0o170000) | mode.bits();
        state.touch();
        Self::write_back(&fs, &mut state)
    }

    /// Updates the owner.
    pub(super) fn set_uid(&self, uid: u32) -> Result<()> {
        let fs = self.fs()?;
        let mut state = self.state.write();
        state.uid = uid;
        state.touch();
        Self::write_back(&fs, &mut state)
    }

    /// Updates the group.
    pub(super) fn set_gid(&self, gid: u32) -> Result<()> {
        let fs = self.fs()?;
        let mut state = self.state.write();
        state.gid = gid;
        state.touch();
        Self::write_back(&fs, &mut state)
    }

    /// Updates a timestamp.
    pub(super) fn set_timestamp(&self, which: TimestampKind, time: Duration) -> Result<()> {
        let fs = self.fs()?;
        let mut state = self.state.write();
        let stamp = Timestamp {
            secs: time.as_secs(),
            nsec: time.subsec_nanos(),
        };
        match which {
            TimestampKind::Atime => state.atime = stamp,
            TimestampKind::Mtime => state.mtime = stamp,
            TimestampKind::Ctime => state.ctime = stamp,
        }
        state.sync_fields_to_node();
        fs.write_data_block(state.node_addr, &state.node)
    }

    /// Creates a child inode and links it into this directory.
    pub(super) fn create_child(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        symlink_target: Option<&str>,
    ) -> Result<Arc<Inode>> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }
        validate_name(name)?;
        if self.inode_type() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the inode is not a directory");
        }

        let child_ino = {
            let mut alloc = fs.write_state();
            let mut state = self.state.write();
            if Self::entries_locked(&fs, &state)?
                .iter()
                .any(|entry| entry.name == name)
            {
                return_errno_with_message!(Errno::EEXIST, "the directory entry exists");
            }

            let (child_ino, node_addr, data_addr) =
                Self::build_new_inode(&fs, &mut alloc, type_, mode, self.ino)?;
            if let Err(err) = insert_dentry(
                &fs,
                &mut alloc,
                &mut state,
                name,
                child_ino,
                dentry_file_type(type_),
            ) {
                let _ = Self::discard_new_inode(&fs, &mut alloc, node_addr, data_addr, child_ino);
                return Err(err);
            }
            if type_ == InodeType::Dir {
                state.links = state.links.saturating_add(1);
            }
            state.touch();
            Self::write_back(&fs, &mut state)?;
            child_ino
        };

        let child = Inode::load(&fs, child_ino)?;
        if let Some(target) = symlink_target {
            child.write_bytes(0, target.as_bytes())?;
        }
        Ok(child)
    }

    /// Removes a child entry, freeing the child when its link count drops to
    /// zero (or unconditionally for directories).
    pub(super) fn remove_child(&self, name: &str, is_dir: bool) -> Result<()> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }
        if name == "." || name == ".." {
            return_errno_with_message!(Errno::EINVAL, "cannot remove a dot entry");
        }

        // Validate the child before taking the parent write lock. Loading an
        // inode while holding a parent lock would invert the lock order, and a
        // corrupted entry pointing back at this directory would self-deadlock.
        let entry = {
            let state = self.state.read();
            Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == name)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?
        };
        if entry.ino == self.ino {
            return_errno_with_message!(Errno::EIO, "directory entry points at its parent");
        }
        if is_dir && entry.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the entry is not a directory");
        }
        if !is_dir && entry.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "the entry is a directory");
        }
        if is_dir {
            let child = Inode::load(&fs, entry.ino)?;
            if !child.is_empty_directory()? {
                return_errno_with_message!(Errno::ENOTEMPTY, "the directory is not empty");
            }
        }

        let child_ino = {
            let mut alloc = fs.write_state();
            let mut state = self.state.write();
            let current = Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == name)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?;
            if current.ino != entry.ino {
                return_errno_with_message!(Errno::EAGAIN, "the entry changed during removal");
            }

            remove_dentry(&fs, &mut alloc, &mut state, name)?
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?;
            if is_dir {
                state.links = state.links.saturating_sub(1);
            }
            state.touch();
            Self::write_back(&fs, &mut state)?;
            entry.ino
        };

        let child = Inode::load(&fs, child_ino)?;
        child.free_if_unlinked()
    }

    fn is_empty_directory(&self) -> Result<bool> {
        Ok(self
            .collect_dentries()?
            .iter()
            .all(|entry| entry.name == "." || entry.name == ".."))
    }

    /// Decrements the link count and frees the inode when it reaches zero.
    fn free_if_unlinked(&self) -> Result<()> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }

        {
            let mut alloc = fs.write_state();
            let mut state = self.state.write();
            if state.type_ != InodeType::Dir {
                state.links = state.links.saturating_sub(1);
                if state.links > 0 {
                    state.touch();
                    return Self::write_back(&fs, &mut state);
                }
            }

            let node_addr = state.node_addr;
            free_all_blocks(&fs, &mut alloc, &mut state)?;
            fs.free_block(&mut alloc, node_addr)?;
            fs.free_nid(self.ino)?;
            alloc.valid_inode_count = alloc.valid_inode_count.saturating_sub(1);
        }

        // Drop the cached object so that a later inode number reuse cannot
        // alias the freed inode.
        fs.evict_inode(self.ino);
        Ok(())
    }
}

/// Scans a directory block for `name` and rewrites the entry's inode number.
fn update_block_entry_ino(block: &mut [u8], name: &str, ino: u32) -> bool {
    let entry_base = DENTRY_BITMAP_SIZE + DENTRY_RESERVED_SIZE;
    let name_base = entry_base + NR_DENTRY_IN_BLOCK * DENTRY_SIZE;
    let mut slot = 0;
    while slot < NR_DENTRY_IN_BLOCK {
        if dentry_slot_free(block, slot) {
            slot += 1;
            continue;
        }
        let entry_offset = entry_base + slot * DENTRY_SIZE;
        let name_len = read_u16(block, entry_offset + 8) as usize;
        let slots = name_len.div_ceil(SLOT_LEN).max(1);
        if name_len == 0 || slot + slots > NR_DENTRY_IN_BLOCK {
            slot += 1;
            continue;
        }
        let name_offset = name_base + slot * SLOT_LEN;
        if &block[name_offset..name_offset + name_len] == name.as_bytes() {
            write_u32(&mut block[entry_offset + 4..], 0, ino);
            return true;
        }
        slot += slots;
    }
    false
}

/// Scans an inline dentry region for `name` and rewrites its inode number.
fn update_inline_entry_ino(
    node: &mut [u8],
    base: usize,
    capacity: usize,
    name: &str,
    ino: u32,
) -> bool {
    let count = capacity * 8 / ((DENTRY_SIZE + SLOT_LEN) * 8 + 1);
    let bitmap_size = count.div_ceil(8);
    let reserved_size = capacity.saturating_sub((DENTRY_SIZE + SLOT_LEN) * count + bitmap_size);
    let entry_base = base + bitmap_size + reserved_size;
    let name_base = entry_base + count * DENTRY_SIZE;

    let mut slot = 0;
    while slot < count {
        if !test_bit_le(&node[base..base + bitmap_size], slot) {
            slot += 1;
            continue;
        }
        let entry_offset = entry_base + slot * DENTRY_SIZE;
        let name_len = read_u16(node, entry_offset + 8) as usize;
        let slots = name_len.div_ceil(SLOT_LEN).max(1);
        if name_len == 0 || slot + slots > count {
            slot += 1;
            continue;
        }
        let name_offset = name_base + slot * SLOT_LEN;
        if &node[name_offset..name_offset + name_len] == name.as_bytes() {
            write_u32(&mut node[entry_offset + 4..], 0, ino);
            return true;
        }
        slot += slots;
    }
    false
}

impl Inode {
    /// Updates the parent recorded by a directory after a rename.
    pub(super) fn update_parent(&self, new_parent: u32) -> Result<()> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }
        let mut state = self.state.write();
        if state.type_ != InodeType::Dir {
            return Ok(());
        }
        state.parent_ino = new_parent;

        if state.inline_flags & F2FS_INLINE_DENTRY != 0 {
            if state.inline_flags & F2FS_INLINE_DOTS == 0 {
                let base = state.inline_base();
                let capacity = state.inline_capacity();
                update_inline_entry_ino(
                    state.node.as_mut_slice(),
                    base,
                    capacity,
                    "..",
                    new_parent,
                );
            }
        } else {
            let block_count = (state.size as usize).div_ceil(F2FS_BLKSIZE);
            for index in 0..block_count {
                let addr = get_data_addr(&fs, &state, index)?;
                if addr == 0 || addr == u32::MAX {
                    continue;
                }
                let mut block = fs.read_data_block(addr)?;
                if update_block_entry_ino(block.as_mut_slice(), "..", new_parent) {
                    fs.write_data_block(addr, &block)?;
                    break;
                }
            }
        }

        Self::write_back(&fs, &mut state)
    }

    /// Moves a directory entry from this directory to `new_dir`.
    pub(super) fn rename_child(
        &self,
        old_name: &str,
        new_dir: &Inode,
        new_name: &str,
        replace: bool,
    ) -> Result<()> {
        let fs = self.fs()?;
        if fs.is_read_only() {
            return_errno_with_message!(Errno::EROFS, "the F2FS volume is mounted read-only");
        }
        let new_fs = new_dir.fs()?;
        if !Arc::ptr_eq(&fs, &new_fs) {
            return_errno_with_message!(Errno::EXDEV, "the directories belong to different mounts");
        }
        validate_name(old_name)?;
        validate_name(new_name)?;
        if self.inode_type() != InodeType::Dir || new_dir.inode_type() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the parent is not a directory");
        }
        if self.ino == new_dir.ino && old_name == new_name {
            return Ok(());
        }

        // Validate the source and target without holding the parent write
        // locks: loading a child inode under a parent lock could invert lock
        // order or deadlock on a corrupted entry pointing back at its parent.
        let source = {
            let state = self.state.read();
            Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == old_name)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?
        };
        if source.ino == self.ino || source.ino == new_dir.ino {
            return_errno_with_message!(Errno::EIO, "directory entry points at its parent");
        }
        let source_ino = source.ino;
        let source_type = source.type_;
        let source_is_dir = source_type == InodeType::Dir;

        let target = {
            let state = new_dir.state.read();
            Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == new_name)
        };
        if let Some(target) = &target {
            if target.ino == source.ino {
                return Ok(());
            }
            if target.ino == self.ino || target.ino == new_dir.ino {
                return_errno_with_message!(Errno::EIO, "directory entry points at its parent");
            }
            if !replace {
                return_errno_with_message!(Errno::EEXIST, "the new path exists");
            }
        }
        let to_free = match &target {
            Some(target) => Self::check_rename_target(&fs, target, source_is_dir)?,
            None => None,
        };

        let mut alloc = fs.write_state();
        if self.ino == new_dir.ino {
            let mut state = self.state.write();
            let current_source = Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == old_name)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?;
            let current_target = Self::entries_locked(&fs, &state)?
                .into_iter()
                .find(|entry| entry.name == new_name);
            Self::check_rename_consistency(&current_source, &current_target, &source, &target)?;

            remove_dentry(&fs, &mut alloc, &mut state, old_name)?;
            if current_target.is_some() {
                remove_dentry(&fs, &mut alloc, &mut state, new_name)?;
            }
            if let Err(err) = insert_dentry(
                &fs,
                &mut alloc,
                &mut state,
                new_name,
                source_ino,
                dentry_file_type(source_type),
            ) {
                let _ = insert_dentry(
                    &fs,
                    &mut alloc,
                    &mut state,
                    old_name,
                    source_ino,
                    dentry_file_type(source_type),
                );
                if let Some(target) = &current_target {
                    let _ = insert_dentry(
                        &fs,
                        &mut alloc,
                        &mut state,
                        new_name,
                        target.ino,
                        dentry_file_type(target.type_),
                    );
                }
                return Err(err);
            }

            if source_is_dir {
                state.links = state.links.saturating_sub(1);
            }
            if current_target.as_ref().map(|t| t.type_ == InodeType::Dir) == Some(true) {
                state.links = state.links.saturating_sub(1);
            }
            if source_is_dir {
                state.links = state.links.saturating_add(1);
            }
            state.touch();
            Self::write_back(&fs, &mut state)?;
        } else {
            let (mut first, mut second) = if self.ino < new_dir.ino {
                (self.state.write(), new_dir.state.write())
            } else {
                (new_dir.state.write(), self.state.write())
            };
            let (old_state, new_state) = if self.ino < new_dir.ino {
                (&mut *first, &mut *second)
            } else {
                (&mut *second, &mut *first)
            };
            let old_state: &mut InodeState = old_state;
            let new_state: &mut InodeState = new_state;

            let current_source = Self::entries_locked(&fs, old_state)?
                .into_iter()
                .find(|entry| entry.name == old_name)
                .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such entry"))?;
            let current_target = Self::entries_locked(&fs, new_state)?
                .into_iter()
                .find(|entry| entry.name == new_name);
            Self::check_rename_consistency(&current_source, &current_target, &source, &target)?;

            remove_dentry(&fs, &mut alloc, old_state, old_name)?;
            if current_target.is_some() {
                remove_dentry(&fs, &mut alloc, new_state, new_name)?;
            }
            if let Err(err) = insert_dentry(
                &fs,
                &mut alloc,
                new_state,
                new_name,
                source_ino,
                dentry_file_type(source_type),
            ) {
                let _ = insert_dentry(
                    &fs,
                    &mut alloc,
                    old_state,
                    old_name,
                    source_ino,
                    dentry_file_type(source_type),
                );
                if let Some(target) = &current_target {
                    let _ = insert_dentry(
                        &fs,
                        &mut alloc,
                        new_state,
                        new_name,
                        target.ino,
                        dentry_file_type(target.type_),
                    );
                }
                return Err(err);
            }

            if source_is_dir {
                old_state.links = old_state.links.saturating_sub(1);
                new_state.links = new_state.links.saturating_add(1);
            }
            if current_target.as_ref().map(|t| t.type_ == InodeType::Dir) == Some(true) {
                new_state.links = new_state.links.saturating_sub(1);
            }
            old_state.touch();
            new_state.touch();
            Self::write_back(&fs, old_state)?;
            Self::write_back(&fs, new_state)?;
        }
        drop(alloc);

        if source_is_dir {
            let moved = Inode::load(&fs, source_ino)?;
            moved.update_parent(new_dir.ino)?;
        }
        if let Some(target_ino) = to_free {
            let target = Inode::load(&fs, target_ino)?;
            target.free_if_unlinked()?;
        }

        Ok(())
    }

    /// Verifies that the directory entries still match the pre-checked state.
    fn check_rename_consistency(
        current_source: &DirEntry,
        current_target: &Option<DirEntry>,
        source: &DirEntry,
        target: &Option<DirEntry>,
    ) -> Result<()> {
        let target_matches = current_target
            .as_ref()
            .map(|entry| (entry.ino, entry.type_))
            == target.as_ref().map(|entry| (entry.ino, entry.type_));
        if current_source.ino != source.ino
            || current_source.type_ != source.type_
            || !target_matches
        {
            return_errno_with_message!(Errno::EAGAIN, "the directory changed during rename");
        }
        Ok(())
    }

    /// Validates that a rename target can be replaced.
    fn check_rename_target(
        fs: &Arc<F2fs>,
        target: &DirEntry,
        source_is_dir: bool,
    ) -> Result<Option<u32>> {
        if source_is_dir {
            if target.type_ != InodeType::Dir {
                return_errno_with_message!(
                    Errno::ENOTDIR,
                    "cannot replace a file with a directory"
                );
            }
            let target_inode = Inode::load(fs, target.ino)?;
            if !target_inode.is_empty_directory()? {
                return_errno_with_message!(Errno::ENOTEMPTY, "the target directory is not empty");
            }
            Ok(Some(target.ino))
        } else {
            if target.type_ == InodeType::Dir {
                return_errno_with_message!(Errno::EISDIR, "cannot replace a directory with a file");
            }
            Ok(Some(target.ino))
        }
    }
}

/// Which timestamp to update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TimestampKind {
    Atime,
    Mtime,
    Ctime,
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > NAME_MAX || name.contains('/') {
        return_errno_with_message!(Errno::EINVAL, "invalid F2FS file name");
    }
    if name == "." || name == ".." {
        return_errno_with_message!(Errno::EINVAL, "invalid F2FS file name");
    }
    Ok(())
}

/// Initializes the footer of a freshly allocated node block.
fn init_node_footer(block: &mut [u8], nid: u32, ino: u32, cp_ver: u64) {
    write_u32(block, NODE_FOOTER_OFFSET, nid);
    write_u32(block, NODE_FOOTER_OFFSET + 4, ino);
    write_u32(block, NODE_FOOTER_OFFSET + 8, 0);
    write_u64(block, NODE_FOOTER_OFFSET + 12, cp_ver);
}

/// Loads a node referenced from an inode's node id array, allocating it if
/// necessary. Returns the node id, block address, and a mutable copy of the
/// block.
fn node_for_update(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    nid_index: usize,
) -> Result<(u32, u32, Block)> {
    let nid = state.nid(nid_index);
    if nid == 0 {
        let (new_nid, addr) = fs.alloc_node(alloc, state_owner(state), None)?;
        let mut block = zeroed_block();
        init_node_footer(
            block.as_mut_slice(),
            new_nid,
            state_owner(state),
            state.cp_ver,
        );
        fs.write_data_block(addr, &block)?;
        state.set_nid(nid_index, new_nid);
        state.blocks = state.blocks.saturating_add(1);
        return Ok((new_nid, addr, block));
    }

    let addr = fs.lookup_nat(nid)?.block_addr;
    let block = fs.read_node(nid)?;
    Ok((nid, addr, block))
}

fn state_owner(state: &InodeState) -> u32 {
    read_u32(state.node.as_slice(), NODE_FOOTER_OFFSET + 4)
}

/// Reads a data block address from a direct node.
fn read_dnode_addr(fs: &Arc<F2fs>, nid: u32, index: usize) -> Result<u32> {
    if nid == 0 {
        return Ok(0);
    }
    let block = fs.read_node(nid)?;
    Ok(read_u32(block.as_slice(), index * 4))
}

/// Reads a data block address through an indirect node.
fn read_indirect_addr(fs: &Arc<F2fs>, indirect_nid: u32, index: usize) -> Result<u32> {
    if indirect_nid == 0 {
        return Ok(0);
    }
    let block = fs.read_node(indirect_nid)?;
    let dnode_nid = read_u32(block.as_slice(), (index / ADDRS_PER_BLOCK) * 4);
    read_dnode_addr(fs, dnode_nid, index % ADDRS_PER_BLOCK)
}

/// Reads a data block address through the double-indirect node.
fn read_double_indirect_addr(fs: &Arc<F2fs>, double_nid: u32, index: usize) -> Result<u32> {
    if double_nid == 0 {
        return Ok(0);
    }
    let block = fs.read_node(double_nid)?;
    let per_indirect = ADDRS_PER_BLOCK * ADDRS_PER_BLOCK;
    let indirect_nid = read_u32(block.as_slice(), (index / per_indirect) * 4);
    read_indirect_addr(fs, indirect_nid, index % per_indirect)
}

/// Returns the physical block address backing file block `index`.
fn get_data_addr(fs: &Arc<F2fs>, state: &InodeState, index: usize) -> Result<u32> {
    let addr_count = state.addr_count();
    if index < addr_count {
        return Ok(state.inode_addr(index));
    }

    let mut index = index - addr_count;
    if index < ADDRS_PER_BLOCK {
        return read_dnode_addr(fs, state.nid(0), index);
    }
    index -= ADDRS_PER_BLOCK;
    if index < ADDRS_PER_BLOCK {
        return read_dnode_addr(fs, state.nid(1), index);
    }
    index -= ADDRS_PER_BLOCK;

    let indirect_capacity = ADDRS_PER_BLOCK * ADDRS_PER_BLOCK;
    if index < indirect_capacity {
        return read_indirect_addr(fs, state.nid(2), index);
    }
    index -= indirect_capacity;
    if index < indirect_capacity {
        return read_indirect_addr(fs, state.nid(3), index);
    }
    index -= indirect_capacity;

    if index < indirect_capacity * ADDRS_PER_BLOCK {
        return read_double_indirect_addr(fs, state.nid(4), index);
    }

    return_errno_with_message!(Errno::EFBIG, "the F2FS file address is out of range");
}

/// Sets the address of file block `index`, allocating node blocks as needed.
/// Returns the parent node id and the offset inside it for summary updates.
fn set_data_addr(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    index: usize,
    addr: u32,
) -> Result<(u32, u16)> {
    let addr_count = state.addr_count();
    if index < addr_count {
        state.set_inode_addr(index, addr);
        return Ok((state_owner(state), index as u16));
    }

    let mut index = index - addr_count;
    if index < ADDRS_PER_BLOCK {
        let (nid, block_addr, mut block) = node_for_update(fs, alloc, state, 0)?;
        write_u32(block.as_mut_slice(), index * 4, addr);
        fs.write_data_block(block_addr, &block)?;
        return Ok((nid, index as u16));
    }
    index -= ADDRS_PER_BLOCK;
    if index < ADDRS_PER_BLOCK {
        let (nid, block_addr, mut block) = node_for_update(fs, alloc, state, 1)?;
        write_u32(block.as_mut_slice(), index * 4, addr);
        fs.write_data_block(block_addr, &block)?;
        return Ok((nid, index as u16));
    }
    index -= ADDRS_PER_BLOCK;

    let indirect_capacity = ADDRS_PER_BLOCK * ADDRS_PER_BLOCK;
    if index < indirect_capacity {
        return set_indirect_addr(fs, alloc, state, 2, index, addr);
    }
    index -= indirect_capacity;
    if index < indirect_capacity {
        return set_indirect_addr(fs, alloc, state, 3, index, addr);
    }
    index -= indirect_capacity;

    if index < indirect_capacity * ADDRS_PER_BLOCK {
        return set_double_indirect_addr(fs, alloc, state, index, addr);
    }

    return_errno_with_message!(Errno::EFBIG, "the F2FS file address is out of range");
}

fn set_indirect_addr(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    nid_index: usize,
    index: usize,
    addr: u32,
) -> Result<(u32, u16)> {
    let (_, indirect_addr, mut indirect) = node_for_update(fs, alloc, state, nid_index)?;
    let slot = index / ADDRS_PER_BLOCK;
    let dnode_nid = read_u32(indirect.as_slice(), slot * 4);
    let (dnode_nid, dnode_addr, mut dnode) = if dnode_nid == 0 {
        let (nid, block_addr) = fs.alloc_node(alloc, state_owner(state), None)?;
        let mut block = zeroed_block();
        init_node_footer(block.as_mut_slice(), nid, state_owner(state), state.cp_ver);
        fs.write_data_block(block_addr, &block)?;
        write_u32(indirect.as_mut_slice(), slot * 4, nid);
        fs.write_data_block(indirect_addr, &indirect)?;
        state.blocks = state.blocks.saturating_add(1);
        (nid, block_addr, block)
    } else {
        let block_addr = fs.lookup_nat(dnode_nid)?.block_addr;
        let block = fs.read_node(dnode_nid)?;
        (dnode_nid, block_addr, block)
    };

    let ofs = index % ADDRS_PER_BLOCK;
    write_u32(dnode.as_mut_slice(), ofs * 4, addr);
    fs.write_data_block(dnode_addr, &dnode)?;
    Ok((dnode_nid, ofs as u16))
}

fn set_double_indirect_addr(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    index: usize,
    addr: u32,
) -> Result<(u32, u16)> {
    let (_, double_addr, mut double) = node_for_update(fs, alloc, state, 4)?;
    let per_indirect = ADDRS_PER_BLOCK * ADDRS_PER_BLOCK;
    let slot = index / per_indirect;
    let mut indirect_nid = read_u32(double.as_slice(), slot * 4);
    if indirect_nid == 0 {
        let (nid, block_addr) = fs.alloc_node(alloc, state_owner(state), None)?;
        let mut block = zeroed_block();
        init_node_footer(block.as_mut_slice(), nid, state_owner(state), state.cp_ver);
        fs.write_data_block(block_addr, &block)?;
        write_u32(double.as_mut_slice(), slot * 4, nid);
        fs.write_data_block(double_addr, &double)?;
        state.blocks = state.blocks.saturating_add(1);
        indirect_nid = nid;
    }

    // `set_indirect_addr()` resolves an indirect node reachable from an
    // `i_nid` slot. Here the second level is not stored in the inode, so the
    // child nid is looked up in the NAT directly before updating its dnode.
    let rest = index % per_indirect;
    let indirect_addr = fs.lookup_nat(indirect_nid)?.block_addr;
    let mut indirect = fs.read_node(indirect_nid)?;
    let dnode_slot = rest / ADDRS_PER_BLOCK;
    let dnode_nid = read_u32(indirect.as_slice(), dnode_slot * 4);
    let (dnode_nid, dnode_addr, mut dnode) = if dnode_nid == 0 {
        let (nid, block_addr) = fs.alloc_node(alloc, state_owner(state), None)?;
        let mut block = zeroed_block();
        init_node_footer(block.as_mut_slice(), nid, state_owner(state), state.cp_ver);
        fs.write_data_block(block_addr, &block)?;
        write_u32(indirect.as_mut_slice(), dnode_slot * 4, nid);
        fs.write_data_block(indirect_addr, &indirect)?;
        state.blocks = state.blocks.saturating_add(1);
        (nid, block_addr, block)
    } else {
        let block_addr = fs.lookup_nat(dnode_nid)?.block_addr;
        let block = fs.read_node(dnode_nid)?;
        (dnode_nid, block_addr, block)
    };

    let ofs = rest % ADDRS_PER_BLOCK;
    write_u32(dnode.as_mut_slice(), ofs * 4, addr);
    fs.write_data_block(dnode_addr, &dnode)?;
    Ok((dnode_nid, ofs as u16))
}

/// Clears the address of file block `index`, returning the old value.
fn clear_data_addr(fs: &Arc<F2fs>, state: &mut InodeState, index: usize) -> Result<u32> {
    let addr_count = state.addr_count();
    if index < addr_count {
        let old = state.inode_addr(index);
        state.set_inode_addr(index, 0);
        return Ok(old);
    }

    let mut index = index - addr_count;
    if index < ADDRS_PER_BLOCK {
        return clear_dnode_addr(fs, state.nid(0), index);
    }
    index -= ADDRS_PER_BLOCK;
    if index < ADDRS_PER_BLOCK {
        return clear_dnode_addr(fs, state.nid(1), index);
    }
    index -= ADDRS_PER_BLOCK;

    let indirect_capacity = ADDRS_PER_BLOCK * ADDRS_PER_BLOCK;
    if index < indirect_capacity {
        return clear_indirect_addr(fs, state.nid(2), index);
    }
    index -= indirect_capacity;
    if index < indirect_capacity {
        return clear_indirect_addr(fs, state.nid(3), index);
    }
    index -= indirect_capacity;

    if index < indirect_capacity * ADDRS_PER_BLOCK {
        let double_nid = state.nid(4);
        if double_nid == 0 {
            return Ok(0);
        }
        let per_indirect = indirect_capacity;
        let block = fs.read_node(double_nid)?;
        let indirect_nid = read_u32(block.as_slice(), (index / per_indirect) * 4);
        return clear_indirect_addr(fs, indirect_nid, index % per_indirect);
    }

    return_errno_with_message!(Errno::EFBIG, "the F2FS file address is out of range");
}

fn clear_dnode_addr(fs: &Arc<F2fs>, nid: u32, index: usize) -> Result<u32> {
    if nid == 0 {
        return Ok(0);
    }
    let addr = fs.lookup_nat(nid)?.block_addr;
    let mut block = fs.read_node(nid)?;
    let old = read_u32(block.as_slice(), index * 4);
    if old != 0 {
        write_u32(block.as_mut_slice(), index * 4, 0);
        fs.write_data_block(addr, &block)?;
    }
    Ok(old)
}

fn clear_indirect_addr(fs: &Arc<F2fs>, indirect_nid: u32, index: usize) -> Result<u32> {
    if indirect_nid == 0 {
        return Ok(0);
    }
    let block = fs.read_node(indirect_nid)?;
    let dnode_nid = read_u32(block.as_slice(), (index / ADDRS_PER_BLOCK) * 4);
    clear_dnode_addr(fs, dnode_nid, index % ADDRS_PER_BLOCK)
}

/// Frees every data block and node block of an inode.
fn free_all_blocks(fs: &Arc<F2fs>, alloc: &mut WriteState, state: &mut InodeState) -> Result<()> {
    let inline_data = state.has_inline_data();
    let inline_dentry = state.inline_flags & F2FS_INLINE_DENTRY != 0;
    if inline_data || inline_dentry {
        // Inline data and inline dentries live in `i_addr`; those bytes are
        // not block addresses and must not be freed.
        if inline_data {
            state.inline_flags &= !(F2FS_INLINE_DATA | F2FS_DATA_EXIST);
        }
        if inline_dentry {
            state.inline_flags &= !(F2FS_INLINE_DENTRY | F2FS_INLINE_DOTS);
        }
    } else {
        let addr_count = state.addr_count();
        for index in 0..addr_count {
            let addr = state.inode_addr(index);
            if addr != 0 && addr != u32::MAX {
                fs.free_block(alloc, addr)?;
            }
            state.set_inode_addr(index, 0);
        }
    }

    for nid_index in 0..NIDS_PER_INODE {
        let nid = state.nid(nid_index);
        if nid != 0 {
            free_node_tree(fs, alloc, nid_index, nid)?;
            state.set_nid(nid_index, 0);
        }
    }

    state.blocks = 1;
    state.size = 0;
    Ok(())
}

/// Frees the data blocks and the node block of a node tree.
fn free_node_tree(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    nid_index: usize,
    nid: u32,
) -> Result<()> {
    let node_addr = fs.lookup_nat(nid)?.block_addr;
    match nid_index {
        0 | 1 => {
            let block = fs.read_node(nid)?;
            for index in 0..ADDRS_PER_BLOCK {
                let addr = read_u32(block.as_slice(), index * 4);
                if addr != 0 && addr != u32::MAX {
                    fs.free_block(alloc, addr)?;
                }
            }
            fs.free_block(alloc, node_addr)?;
            fs.free_nid(nid)?;
        }
        2 | 3 => {
            let block = fs.read_node(nid)?;
            for index in 0..ADDRS_PER_BLOCK {
                let child = read_u32(block.as_slice(), index * 4);
                if child != 0 {
                    free_node_tree(fs, alloc, 0, child)?;
                }
            }
            fs.free_block(alloc, node_addr)?;
            fs.free_nid(nid)?;
        }
        4 => {
            let block = fs.read_node(nid)?;
            for index in 0..ADDRS_PER_BLOCK {
                let child = read_u32(block.as_slice(), index * 4);
                if child != 0 {
                    free_node_tree(fs, alloc, 2, child)?;
                }
            }
            fs.free_block(alloc, node_addr)?;
            fs.free_nid(nid)?;
        }
        _ => {}
    }
    Ok(())
}

/// Frees only the node blocks of an inode after its data blocks were freed.
fn free_node_blocks(fs: &Arc<F2fs>, alloc: &mut WriteState, state: &mut InodeState) -> Result<()> {
    for nid_index in 0..NIDS_PER_INODE {
        let nid = state.nid(nid_index);
        if nid != 0 {
            free_node_tree(fs, alloc, nid_index, nid)?;
            state.set_nid(nid_index, 0);
            state.blocks = state.blocks.saturating_sub(1);
        }
    }
    Ok(())
}

fn set_dentry_bit(block: &mut [u8], slot: usize) {
    block[slot / 8] |= 1 << (slot % 8);
}

fn clear_dentry_bit(block: &mut [u8], slot: usize) {
    block[slot / 8] &= !(1 << (slot % 8));
}

fn dentry_slot_free(block: &[u8], slot: usize) -> bool {
    !test_bit_le(block, slot)
}

/// Writes a directory entry into a dentry block.
fn write_dentry(block: &mut [u8], slot: usize, name: &str, ino: u32, file_type: u8) -> Result<()> {
    let slots = name.len().div_ceil(SLOT_LEN);
    if slots == 0 || slot + slots > NR_DENTRY_IN_BLOCK {
        return_errno_with_message!(Errno::ENOSPC, "no room for the directory entry");
    }

    let entry_offset = DENTRY_BITMAP_SIZE + DENTRY_RESERVED_SIZE + slot * DENTRY_SIZE;
    let name_offset = DENTRY_BITMAP_SIZE
        + DENTRY_RESERVED_SIZE
        + NR_DENTRY_IN_BLOCK * DENTRY_SIZE
        + slot * SLOT_LEN;
    let name_bytes = name.as_bytes();

    let hash = if name == "." || name == ".." {
        0
    } else {
        dentry_hash(name_bytes)
    };
    write_u32(block, entry_offset, hash);
    write_u32(block, entry_offset + 4, ino);
    write_u16(block, entry_offset + 8, name_bytes.len() as u16);
    block[entry_offset + 10] = file_type;

    for offset in 0..(slots * SLOT_LEN) {
        block[name_offset + offset] = name_bytes.get(offset).copied().unwrap_or(0);
    }
    for index in slot..slot + slots {
        set_dentry_bit(block, index);
    }

    Ok(())
}

fn find_free_slots(block: &[u8], slots: usize) -> Option<usize> {
    if slots > NR_DENTRY_IN_BLOCK {
        return None;
    }
    let mut run = 0;
    for slot in 0..NR_DENTRY_IN_BLOCK {
        if dentry_slot_free(block, slot) {
            run += 1;
            if run == slots {
                return Some(slot + 1 - slots);
            }
        } else {
            run = 0;
        }
    }
    None
}

fn insert_into_block(block: &mut [u8], name: &str, ino: u32, file_type: u8) -> Result<()> {
    let slots = name.len().div_ceil(SLOT_LEN);
    let slot = find_free_slots(block, slots)
        .ok_or_else(|| Error::with_message(Errno::ENOSPC, "directory block is full"))?;
    write_dentry(block, slot, name, ino, file_type)
}

/// Moves inline file data into a regular data block.
fn materialize_inline_data(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
) -> Result<()> {
    let capacity = state.inline_capacity();
    let size = (state.size as usize).min(capacity);
    let base = state.inline_base();
    let mut block = zeroed_block();
    block[..size].copy_from_slice(&state.node[base..base + size]);

    let data_addr = fs.alloc_block(alloc, BlockKind::Data)?;
    let (nid, ofs) = set_data_addr(fs, alloc, state, 0, data_addr)?;
    fs.write_summary(data_addr, nid, ofs, BlockKind::Data)?;
    fs.write_data_block(data_addr, &block)?;

    state.blocks = state.blocks.saturating_add(1);
    state.inline_flags &= !(F2FS_INLINE_DATA | F2FS_DATA_EXIST);
    Ok(())
}

/// Converts an inline directory to a block-backed directory.
fn convert_inline_dir(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
) -> Result<()> {
    let mut entries = Inode::entries_locked(fs, state)?;
    if !entries.iter().any(|entry| entry.name == ".") {
        entries.insert(
            0,
            DirEntry {
                name: ".".to_string(),
                ino: state.ino,
                type_: InodeType::Dir,
            },
        );
    }
    if !entries.iter().any(|entry| entry.name == "..") {
        entries.insert(
            1,
            DirEntry {
                name: "..".to_string(),
                ino: state.parent_ino,
                type_: InodeType::Dir,
            },
        );
    }

    let data_addr = fs.alloc_block(alloc, BlockKind::Data)?;
    let (nid, ofs) = set_data_addr(fs, alloc, state, 0, data_addr)?;
    fs.write_summary(data_addr, nid, ofs, BlockKind::Data)?;
    state.blocks = state.blocks.saturating_add(1);
    state.size = F2FS_BLKSIZE as u64;
    state.inline_flags &= !(F2FS_INLINE_DENTRY | F2FS_INLINE_DOTS);

    let mut block = zeroed_block();
    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            // Dot entries are known to fit.
            insert_into_block(
                block.as_mut_slice(),
                &entry.name,
                entry.ino,
                dentry_file_type(InodeType::Dir),
            )?;
        } else {
            let _ = insert_into_block(
                block.as_mut_slice(),
                &entry.name,
                entry.ino,
                dentry_file_type(entry.type_),
            );
        }
    }
    fs.write_data_block(data_addr, &block)?;
    Inode::write_back(fs, state)
}

/// Inserts a directory entry into a directory.
fn insert_dentry(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    name: &str,
    ino: u32,
    file_type: u8,
) -> Result<()> {
    if state.inline_flags & F2FS_INLINE_DENTRY != 0 {
        convert_inline_dir(fs, alloc, state)?;
    }

    let block_count = (state.size as usize).div_ceil(F2FS_BLKSIZE);
    for index in 0..block_count {
        let addr = get_data_addr(fs, state, index)?;
        if addr == 0 || addr == u32::MAX {
            continue;
        }
        let mut block = fs.read_data_block(addr)?;
        if find_free_slots(block.as_slice(), name.len().div_ceil(SLOT_LEN)).is_some() {
            insert_into_block(block.as_mut_slice(), name, ino, file_type)?;
            fs.write_data_block(addr, &block)?;
            return Ok(());
        }
    }

    let new_addr = fs.alloc_block(alloc, BlockKind::Data)?;
    let (nid, ofs) = set_data_addr(fs, alloc, state, block_count, new_addr)?;
    fs.write_summary(new_addr, nid, ofs, BlockKind::Data)?;
    state.blocks = state.blocks.saturating_add(1);
    state.size = state.size.saturating_add(F2FS_BLKSIZE as u64);
    let mut block = zeroed_block();
    insert_into_block(block.as_mut_slice(), name, ino, file_type)?;
    fs.write_data_block(new_addr, &block)?;
    Inode::write_back(fs, state)
}

/// Removes a directory entry, returning its inode number.
fn remove_dentry(
    fs: &Arc<F2fs>,
    alloc: &mut WriteState,
    state: &mut InodeState,
    name: &str,
) -> Result<Option<u32>> {
    if state.inline_flags & F2FS_INLINE_DENTRY != 0 {
        convert_inline_dir(fs, alloc, state)?;
    }

    let block_count = (state.size as usize).div_ceil(F2FS_BLKSIZE);
    for index in 0..block_count {
        let addr = get_data_addr(fs, state, index)?;
        if addr == 0 || addr == u32::MAX {
            continue;
        }
        let mut block = fs.read_data_block(addr)?;
        let bitmap_size = DENTRY_BITMAP_SIZE;
        let entry_base = bitmap_size + DENTRY_RESERVED_SIZE;
        let name_base = entry_base + NR_DENTRY_IN_BLOCK * DENTRY_SIZE;
        let mut slot = 0;
        while slot < NR_DENTRY_IN_BLOCK {
            if dentry_slot_free(block.as_slice(), slot) {
                slot += 1;
                continue;
            }
            let entry_offset = entry_base + slot * DENTRY_SIZE;
            let name_len = read_u16(block.as_slice(), entry_offset + 8) as usize;
            let slots = name_len.div_ceil(SLOT_LEN).max(1);
            if name_len == 0 || slot + slots > NR_DENTRY_IN_BLOCK {
                slot += 1;
                continue;
            }
            let name_offset = name_base + slot * SLOT_LEN;
            if &block[name_offset..name_offset + name_len] == name.as_bytes() {
                let ino = read_u32(block.as_slice(), entry_offset + 4);
                for bit in slot..slot + slots {
                    clear_dentry_bit(block.as_mut_slice(), bit);
                }
                fs.write_data_block(addr, &block)?;
                return Ok(Some(ino));
            }
            slot += slots;
        }
    }

    Ok(None)
}
