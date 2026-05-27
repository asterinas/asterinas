// SPDX-License-Identifier: MPL-2.0

//! Extended attributes for the ext2 filesystem.
//!
//! Extended attributes are stored on disk blocks allocated outside of any inode.
//! The `i_file_acl` field of the on-disk inode points to this allocated block.
//! An inode with no attributes has `i_file_acl == 0` and no block is allocated.
//!
//! # On-disk layout
//!
//! The EA block is exactly one filesystem block and is structured as follows:
//!
//! ```text
//!   +------------------+
//!   | header (32B)     |
//!   | entry 1 header   |  ┐
//!   | entry 1 name     |  ├ entry 1
//!   | entry 2 header   |  ┐  |
//!   | entry 2 name     |  ├ entry 2  | growing downwards
//!   | entry 3 header   |  ┐  |       v
//!   | entry 3 name     |  ├ entry 3
//!   | (terminator)     |
//!   | (free space)     |
//!   | . . .            |
//!   | value 3          |  ┐
//!   | value 1          |  │ growing upwards
//!   | value 2          |  ┘
//!   +------------------+
//! ```
//!
//! The block header is followed by multiple entry descriptors. Each entry
//! consists of an `XattrEntry` header immediately followed by the name
//! bytes (whose length is given by `name_len`). Entries are variable in size,
//! aligned to 4-byte (XATTR_ALIGN) boundaries, and sorted by
//! `(name_index as u8, name_len, name bytes)` so that two extended attribute
//! blocks can be compared efficiently.
//!
//! Attribute values are aligned to the end of the block, stored in no
//! specific order. They are also padded to 4-byte boundaries, with no
//! additional gaps between them.
//!
//! Each entry descriptor stores:
//! - `name_index` — a `XattrNameIndex` byte that encodes the namespace
//!   prefix (e.g., `user.`, `security.`).
//! - `name_len` — byte length of the suffix that follows the header.
//! - `value_offset` — byte offset of the value from the start of the block.
//! - `value_len` — byte length of the value.
//! - `hash` — per-entry hash covering name and value (currently always 0;
//!   block sharing via mb_cache is not implemented).
//!
//! The entry list is terminated by an `XattrEntry` with `name_len == 0`
//! (a 16-byte zeroed header), matching Linux's `IS_LAST_ENTRY` check.
//!
//! # Namespace mapping
//!
//! The VFS `XattrNamespace` is mapped to ext2's compact `XattrNameIndex`
//! byte. The full attribute name stored in the VFS layer (e.g., `user.foo`)
//! is split at the namespace boundary before storage: the index is written to
//! `name_index` and only the suffix (`foo`) is stored in `name_len` bytes
//! after the header. The prefix is reconstructed on read.
//!
//! # Locking
//!
//! All mutable state is held in `XattrCache` behind an internal `RwMutex`.
//! Public methods on `Xattr` acquire the lock automatically for the duration
//! of each operation. Callers (e.g. `Inode`) never see the lock and must not
//! hold `Inode::inner` while calling into `Xattr`. When a mutation changes the
//! block number (allocation or free), the caller reads the new `Xattr::bid`
//! after the mutation returns and then acquires `inner` to update
//! `InodeDesc.file_acl`.
//!
//! See the locking hierarchy documented on `Inode`.

use core::cmp::Ordering;

use super::{fs::Ext2, inode::Inode, prelude::*};
use crate::fs::vfs::xattr::{XattrName, XattrNamespace, XattrSetFlags};

const XATTR_NBLOCKS: usize = 1;
const XATTR_MAGIC: u32 = 0xEA02_0000;
const XATTR_ALIGN: usize = 4;
const XATTR_ROUND: usize = XATTR_ALIGN - 1;
const XATTR_HEADER_SIZE: usize = size_of::<XattrHeader>();
const XATTR_ENTRY_HEADER_SIZE: usize = size_of::<XattrEntry>();
const XATTR_TERMINATOR_SIZE: usize = size_of::<u32>();
const XATTR_ENTRY_VALUE_GAP: usize = XATTR_ALIGN;

fn xattr_entry_len(name_len: usize) -> usize {
    (name_len + XATTR_ENTRY_HEADER_SIZE + XATTR_ROUND) & !XATTR_ROUND
}

fn xattr_value_size(size: usize) -> usize {
    (size + XATTR_ROUND) & !XATTR_ROUND
}

/// External extended-attribute block state for one inode.
#[derive(Debug)]
pub(super) struct Xattr {
    cache: RwMutex<XattrCache>,
}

impl Xattr {
    /// Creates a new xattr handle for the block referenced by `InodeDesc.file_acl`.
    pub(super) fn new(bid: Ext2Bid, inode: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        Self {
            cache: RwMutex::new(XattrCache {
                block_buf: None,
                entries: Vec::new(),
                bid,
                dirty: false,
                inode,
                fs,
            }),
        }
    }

    /// Returns the current xattr block number for `InodeDesc.file_acl`.
    pub(super) fn bid(&self) -> Ext2Bid {
        self.cache.read().bid()
    }

    /// Creates or replaces one extended attribute.
    ///
    /// Allocates an xattr block if the inode does not have one yet.
    pub(super) fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        let value_len = value_reader.remain();
        if value_len > BLOCK_SIZE {
            return_errno_with_message!(Errno::ERANGE, "xattr value is too large");
        }

        let mut value = vec![0u8; value_len];
        if value_len > 0 {
            value_reader.read_fallible(&mut VmWriter::from(value.as_mut_slice()))?;
        }

        self.cache
            .write()
            .set_entry(target_index, target_name, value, flags)
    }

    /// Reads one extended attribute value.
    ///
    /// When `vm_writer.avail() == 0`, returns the value length without copying
    /// bytes to userspace.
    pub(super) fn get_xattr(&self, name: XattrName, vm_writer: &mut VmWriter) -> Result<usize> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        self.cache
            .write()
            .get_entry(target_index, &target_name, vm_writer)
    }

    /// Lists extended-attribute names in one namespace.
    ///
    /// When `list_writer.avail() == 0`, returns the required list size without
    /// copying bytes to userspace.
    pub(super) fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.cache.write().list_entries(namespace, list_writer)
    }

    /// Removes one extended attribute.
    ///
    /// Frees the xattr block if this was the last entry in it.
    pub(super) fn remove_xattr(&self, name: XattrName) -> Result<()> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        self.cache.write().remove_entry(target_index, &target_name)
    }

    /// Frees the xattr block entirely (called during inode eviction).
    pub(super) fn delete_xattr_block(&self) -> Result<()> {
        self.cache.write().free_and_invalidate()
    }

    /// Writes the dirty xattr block back to disk.
    pub(super) fn flush(&self) -> Result<()> {
        self.cache.write().flush()
    }

    /// Parses a VFS [`XattrName`] into the ext2 `(name_index, name_suffix)` pair.
    fn parse_target_name(name: XattrName) -> Result<(XattrNameIndex, Vec<u8>)> {
        let name_index = XattrNameIndex::from(name.namespace());
        let stripped_name = name_index.strip_prefix(name.full_name())?;
        if stripped_name.len() > u8::MAX as usize {
            return_errno_with_message!(Errno::ERANGE, "xattr name is too long");
        }
        Ok((name_index, stripped_name.as_bytes().to_vec()))
    }
}

/// Persistent state for an ext2 xattr block.
///
/// Tracks the block buffer, block number, decoded entries, and dirty flag for
/// one inode.
#[derive(Debug)]
struct XattrCache {
    block_buf: Option<USegment>,
    /// Lazily decoded entries.
    ///
    /// Always populated after the first mutation or query; empty only for an
    /// inode that has never had xattrs and has `bid == 0`.
    entries: Vec<XattrEntryData>,
    bid: Ext2Bid,
    dirty: bool,
    inode: Weak<Inode>,
    fs: Weak<Ext2>,
}

impl XattrCache {
    fn bid(&self) -> Ext2Bid {
        self.bid
    }

    fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "ext2 instance is unavailable"))
    }

    fn inode(&self) -> Result<Arc<Inode>> {
        self.inode
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "inode instance is unavailable"))
    }

    fn set_entry(
        &mut self,
        target_index: XattrNameIndex,
        target_name: Vec<u8>,
        value: Vec<u8>,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.load()?;

        let (found_idx, insert_at) =
            Self::find_entry_position(&self.entries, target_index, &target_name);

        if found_idx.is_some() {
            if flags.contains(XattrSetFlags::CREATE_ONLY) {
                return_errno_with_message!(Errno::EEXIST, "the target xattr already exists");
            }
        } else if flags.contains(XattrSetFlags::REPLACE_ONLY) {
            return_errno_with_message!(Errno::ENODATA, "the target xattr does not exist");
        }

        if let Some(idx) = found_idx {
            self.entries[idx].value = value;
        } else {
            self.entries.insert(
                insert_at,
                XattrEntryData {
                    name_index: target_index,
                    name: target_name,
                    value,
                },
            );
        }

        if self.bid == 0 {
            self.alloc_block()?;
        }
        self.dirty = true;
        Ok(())
    }

    fn get_entry(
        &mut self,
        target_index: XattrNameIndex,
        target_name: &[u8],
        vm_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.load()?;

        let (found_idx, _) = Self::find_entry_position(&self.entries, target_index, target_name);
        let idx = found_idx.ok_or_else(|| {
            Error::with_message(Errno::ENODATA, "the target xattr does not exist")
        })?;
        let value = self.entries[idx].value.as_slice();

        if vm_writer.avail() == 0 {
            return Ok(value.len());
        }
        if value.len() > vm_writer.avail() {
            return_errno_with_message!(Errno::ERANGE, "the xattr value buffer is too small");
        }

        vm_writer.write_fallible(&mut VmReader::from(value))?;
        Ok(value.len())
    }

    fn remove_entry(&mut self, target_index: XattrNameIndex, target_name: &[u8]) -> Result<()> {
        self.load()?;

        let (found_idx, _) = Self::find_entry_position(&self.entries, target_index, target_name);
        let Some(found_idx) = found_idx else {
            return_errno_with_message!(Errno::ENODATA, "the target xattr does not exist");
        };
        self.entries.remove(found_idx);

        if self.entries.is_empty() {
            self.free_and_invalidate()?;
            return Ok(());
        }

        self.dirty = true;
        Ok(())
    }

    // TODO: The VFS `Inode::list_xattr` trait passes a single namespace,
    // but `sys_listxattr` only queries one namespace (Trusted for root,
    // User otherwise), which is wrong — Linux returns all visible
    // namespaces.  As a workaround we replicate the old ext2 behavior:
    // when the caller passes User, list only user.* entries; otherwise
    // list all entries (matching root visibility in Linux).  The proper
    // fix is to enumerate all visible namespaces in the syscall layer
    // (see `kernel/src/syscall/listxattr.rs`).
    fn list_entries(
        &mut self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.load()?;

        let buffer_size = list_writer.avail();
        let mut total_list_size = 0usize;
        let mut remaining_size = buffer_size;

        for entry in &self.entries {
            if namespace == XattrNamespace::User
                && entry.name_index.namespace() != XattrNamespace::User
            {
                continue;
            }

            let prefix = entry.name_index.prefix().as_bytes();
            let list_entry_size = prefix.len() + entry.name.len() + 1;
            total_list_size += list_entry_size;

            if buffer_size > 0 {
                if list_entry_size > remaining_size {
                    return_errno_with_message!(Errno::ERANGE, "the xattr list buffer is too small");
                }
                list_writer.write_fallible(&mut VmReader::from(prefix))?;
                list_writer.write_fallible(&mut VmReader::from(entry.name.as_slice()))?;
                list_writer.write_fallible(&mut VmReader::from(&[0u8][..]))?;
                remaining_size -= list_entry_size;
            }
        }

        if buffer_size > 0 {
            Ok(buffer_size - remaining_size)
        } else {
            Ok(total_list_size)
        }
    }

    /// Finds the position of an entry key in the sorted entry list.
    fn find_entry_position(
        entries: &[XattrEntryData],
        target_index: XattrNameIndex,
        target_name: &[u8],
    ) -> (Option<usize>, usize) {
        let mut found_idx = None;
        let mut insert_at = entries.len();

        for (idx, entry) in entries.iter().enumerate() {
            match cmp_entry_key(entry.name_index, &entry.name, target_index, target_name) {
                Ordering::Less => continue,
                Ordering::Equal => {
                    found_idx = Some(idx);
                    insert_at = idx;
                    break;
                }
                Ordering::Greater => {
                    insert_at = idx;
                    break;
                }
            }
        }

        (found_idx, insert_at)
    }

    /// Loads the EA block from disk if it is not already loaded.
    ///
    /// For inodes with `bid == 0` (no block allocated), leaves the entry list
    /// empty.
    fn load(&mut self) -> Result<()> {
        if self.block_buf.is_some() {
            return Ok(());
        }
        if self.bid == 0 {
            return Ok(());
        }

        let fs = self.fs()?;
        let block_buf = Self::alloc_block_buffer()?;

        let bio_segment = BioSegment::new_from_segment(block_buf.clone(), BioDirection::FromDevice);
        fs.read_blocks(self.bid, bio_segment)?;

        Self::validate_block(&block_buf)?;
        let entries = Self::parse_entries_from_block(&block_buf)?;
        self.entries = entries;
        self.block_buf = Some(block_buf);
        Ok(())
    }

    fn alloc_block(&mut self) -> Result<()> {
        debug_assert!(self.bid == 0);

        let fs = self.fs()?;
        let inode = self.inode()?;
        let goal = {
            let sb = fs.super_block();
            sb.group_first_block_no(inode.block_group_idx())
        };
        let range = fs.alloc_blocks(1, goal)?;
        self.bid = range.start;
        if self.block_buf.is_none() {
            self.block_buf = Some(Self::alloc_block_buffer()?);
        }
        Ok(())
    }

    /// Frees the EA block (if allocated) and invalidates the cache.
    fn free_and_invalidate(&mut self) -> Result<()> {
        if self.bid != 0 {
            let fs = self.fs()?;
            fs.free_blocks(self.bid, 1)?;
        }
        self.bid = 0;
        self.block_buf = None;
        self.entries = Vec::new();
        self.dirty = false;
        Ok(())
    }

    /// Writes the dirty xattr block back to disk.
    fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        if self.bid == 0 {
            self.dirty = false;
            return Ok(());
        }

        let fs = self.fs()?;
        let block_buf = self.block_buf.as_ref().unwrap();
        self.write_entries_to_segment(block_buf)?;

        let bio_segment = BioSegment::new_from_segment(block_buf.clone(), BioDirection::ToDevice);
        fs.write_blocks(self.bid, bio_segment)?;
        self.dirty = false;
        Ok(())
    }

    /// Serializes the xattr block into its on-disk layout.
    ///
    /// The block contains an `XattrHeader`, raw `XattrEntry` records with
    /// inline names, value bytes packed from the end of the block, and a
    /// terminator entry.
    fn write_entries_to_segment(&self, block: &USegment) -> Result<()> {
        let entries = &self.entries;

        block
            .fill_zeros(0, BLOCK_SIZE)
            .map_err(|(error, _)| error)?;

        let header = XattrHeader {
            magic: XATTR_MAGIC,
            ref_count: 1,
            nblocks: XATTR_NBLOCKS as u32,
            hash: 0,
            reserved: [0u32; 4],
        };
        block.write_val(0, &header)?;

        let mut entry_cursor = XATTR_HEADER_SIZE;
        let mut value_cursor = BLOCK_SIZE;

        for entry in entries {
            let entry_padded_size = xattr_entry_len(entry.name.len());
            let value_padded_size = xattr_value_size(entry.value.len());

            value_cursor = value_cursor.checked_sub(value_padded_size).ok_or_else(|| {
                Error::with_message(Errno::ENOSPC, "insufficient xattr value space")
            })?;

            if entry_cursor + entry_padded_size + XATTR_ENTRY_VALUE_GAP > value_cursor {
                return_errno_with_message!(Errno::ENOSPC, "xattr entry/value regions overlap");
            }

            if !entry.value.is_empty() {
                block.write_bytes(value_cursor, &entry.value)?;
            }

            let raw_entry = XattrEntry {
                name_len: entry.name.len() as u8,
                name_index: entry.name_index as u8,
                value_offset: if entry.value.is_empty() {
                    0
                } else {
                    value_cursor as u16
                },
                value_block: 0,
                value_len: entry.value.len() as u32,
                hash: 0,
            };
            block.write_val(entry_cursor, &raw_entry)?;

            let name_offset = entry_cursor + XATTR_ENTRY_HEADER_SIZE;
            block.write_bytes(name_offset, &entry.name)?;

            entry_cursor += entry_padded_size;
        }

        let terminator = XattrEntry {
            name_len: 0,
            name_index: 0,
            value_offset: 0,
            value_block: 0,
            value_len: 0,
            hash: 0,
        };
        block.write_val(entry_cursor, &terminator)?;

        Ok(())
    }

    fn alloc_block_buffer() -> Result<USegment> {
        let segment = FrameAllocOptions::new().zeroed(true).alloc_segment(1)?;
        Ok(segment.into())
    }

    fn validate_header(block_buf: &USegment) -> Result<()> {
        let header = block_buf.read_val::<XattrHeader>(0)?;
        if header.magic != XATTR_MAGIC || header.nblocks != XATTR_NBLOCKS as u32 {
            return_errno_with_message!(Errno::EIO, "invalid xattr header");
        }
        Ok(())
    }

    fn validate_entry(entry: &XattrEntry, offset: usize) -> Result<()> {
        let entry_len = xattr_entry_len(entry.name_len as usize);
        let next_entry_offset = offset + entry_len;
        if next_entry_offset >= BLOCK_SIZE {
            return_errno_with_message!(Errno::EIO, "xattr entry overflows block");
        }
        if entry.value_block != 0 {
            return_errno_with_message!(Errno::EIO, "xattr external value blocks are not supported");
        }

        let value_size = entry.value_len as usize;
        let value_offset = entry.value_offset as usize;
        if value_size > BLOCK_SIZE {
            return_errno_with_message!(Errno::EIO, "xattr value range is out of block bounds");
        }
        let value_end = value_offset + value_size;
        if value_end > BLOCK_SIZE {
            return_errno_with_message!(Errno::EIO, "xattr value range is out of block bounds");
        }
        Ok(())
    }

    fn validate_block(block_buf: &USegment) -> Result<()> {
        Self::validate_header(block_buf)?;

        let mut offset = XATTR_HEADER_SIZE;
        loop {
            if offset + XATTR_TERMINATOR_SIZE > BLOCK_SIZE {
                return_errno_with_message!(Errno::EIO, "xattr entry terminator is missing");
            }

            let marker = block_buf.read_val::<u32>(offset)?;
            if marker == 0 {
                return Ok(());
            }

            let entry = block_buf.read_val::<XattrEntry>(offset)?;
            let _ = XattrNameIndex::try_from(entry.name_index)
                .map_err(|_| Error::with_message(Errno::EIO, "invalid xattr name index on disk"))?;
            Self::validate_entry(&entry, offset)?;
            offset += xattr_entry_len(entry.name_len as usize);
        }
    }

    fn parse_entries_from_block(block_buf: &USegment) -> Result<Vec<XattrEntryData>> {
        let mut entries: Vec<XattrEntryData> = Vec::new();
        let mut offset = XATTR_HEADER_SIZE;
        loop {
            if offset + XATTR_TERMINATOR_SIZE > BLOCK_SIZE {
                return_errno_with_message!(Errno::EIO, "xattr entry terminator is missing");
            }

            let marker = block_buf.read_val::<u32>(offset)?;
            if marker == 0 {
                break;
            }

            let entry = block_buf.read_val::<XattrEntry>(offset)?;
            let name_index = XattrNameIndex::try_from(entry.name_index)
                .map_err(|_| Error::with_message(Errno::EIO, "invalid xattr name index on disk"))?;
            Self::validate_entry(&entry, offset)?;

            let name_len = entry.name_len as usize;
            let mut name = vec![0u8; name_len];
            block_buf.read_bytes(offset + XATTR_ENTRY_HEADER_SIZE, &mut name)?;

            // Entries must be strictly sorted by their key.
            if let Some(prev_entry) = entries.last() {
                let ordering =
                    cmp_entry_key(prev_entry.name_index, &prev_entry.name, name_index, &name);
                if ordering != Ordering::Less {
                    return_errno_with_message!(Errno::EIO, "xattr entries are not strictly sorted");
                }
            }

            let value_len = entry.value_len as usize;
            let mut value = vec![0u8; value_len];
            if value_len > 0 {
                block_buf.read_bytes(entry.value_offset as usize, &mut value)?;
            }

            entries.push(XattrEntryData {
                name_index,
                name,
                value,
            });

            offset += xattr_entry_len(name_len);
        }

        Ok(entries)
    }
}
/// The ext2 xattr namespace index stored in `name_index`.
///
/// Determines the namespace prefix prepended to the attribute name
/// (e.g., `user.`, `security.`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum XattrNameIndex {
    User = 1,
    PosixAclAccess = 2,
    PosixAclDefault = 3,
    Trusted = 4,
    Lustre = 5,
    Security = 6,
}

impl From<XattrNamespace> for XattrNameIndex {
    fn from(namespace: XattrNamespace) -> Self {
        match namespace {
            XattrNamespace::User => Self::User,
            XattrNamespace::Trusted => Self::Trusted,
            XattrNamespace::Security => Self::Security,
            XattrNamespace::System => {
                // TODO: POSIX ACL xattrs are not implemented yet.
                Self::PosixAclAccess
            }
        }
    }
}

impl XattrNameIndex {
    /// Returns the namespace prefix string (e.g. `"user."`).
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Self::User => "user.",
            Self::PosixAclAccess => "system.posix_acl_access.",
            Self::PosixAclDefault => "system.posix_acl_default.",
            Self::Trusted => "trusted.",
            Self::Lustre => "lustre.",
            Self::Security => "security.",
        }
    }

    /// Strips the namespace prefix from `full_name` and returns the remainder.
    pub(super) fn strip_prefix(self, full_name: &str) -> Result<&str> {
        full_name.strip_prefix(self.prefix()).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "xattr name does not match namespace-specific prefix",
            )
        })
    }

    /// Maps the on-disk `name_index` byte back to a VFS [`XattrNamespace`].
    fn namespace(self) -> XattrNamespace {
        match self {
            Self::User => XattrNamespace::User,
            Self::Trusted => XattrNamespace::Trusted,
            Self::Security => XattrNamespace::Security,
            Self::PosixAclAccess | Self::PosixAclDefault | Self::Lustre => XattrNamespace::System,
        }
    }
}

/// In-memory decoded xattr entry used as the working representation during mutations.
///
/// # Differences from Linux's `ext2_xattr_entry`
///
/// - **`name` is a separate `Vec<u8>`**: In the on-disk layout, the name
///   bytes follow immediately after the `XattrEntry` header.  Because Rust
///   does not permit safe mutable borrows of sub-ranges of a buffer, we
///   store the name as an owned `Vec<u8>` during the in-memory phase.  It
///   is written back to the correct offset in the block buffer on flush.
/// - **`hash` is not stored here**: The per-entry hash in `XattrEntry::hash`
///   is always written as zero.  Block sharing via `mb_cache` (which would
///   require hash-based deduplication) is not implemented; `ref_count` is
///   always 1.
#[derive(Clone, Debug)]
struct XattrEntryData {
    name_index: XattrNameIndex,
    name: Vec<u8>,
    value: Vec<u8>,
}

/// On-disk header of an ext2 extended-attribute block (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct XattrHeader {
    magic: u32,
    // TODO: support block_sharing, currently the `ref_count` is always 1
    ref_count: u32,
    nblocks: u32,
    hash: u32,
    reserved: [u32; 4],
}

/// On-disk extended-attribute entry header.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct XattrEntry {
    name_len: u8,
    name_index: u8,
    value_offset: u16,
    value_block: u32,
    value_len: u32,
    hash: u32,
}

/// Compare two xattr keys for sort order.
///
/// Ordering is `(name_index as u8, name_len, name bytes)`, matching Linux.
fn cmp_entry_key(
    lhs_index: XattrNameIndex,
    lhs_name: &[u8],
    rhs_index: XattrNameIndex,
    rhs_name: &[u8],
) -> Ordering {
    (lhs_index as u8)
        .cmp(&(rhs_index as u8))
        .then(lhs_name.len().cmp(&rhs_name.len()))
        .then(lhs_name.cmp(rhs_name))
}
