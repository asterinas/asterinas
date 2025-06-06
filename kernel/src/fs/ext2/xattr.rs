// SPDX-License-Identifier: MPL-2.0

use ostd::mm::UntypedMem;

use super::{block_ptr::Ext2Bid, prelude::*, Ext2, Inode};
use crate::fs::utils::{XattrName, XattrNamespace, XattrSetFlags, XATTR_NAME_MAX_LEN};

const EXT2_XATTR_MAGIC: u32 = 0xEA020000;

/// The xattr header of an ext2 inode, organized
/// at the beginning of an xattr block.
#[repr(C)]
#[derive(Clone, Copy, Pod, Debug)]
struct XattrHeader {
    magic: u32,
    ref_count: u32,
    nblocks: u32,
    hash: u32,
    reserved: [u32; 4],
}

const XATTR_HEADER_SIZE: usize = size_of::<XattrHeader>();
const XATTR_ALIGN: usize = 4;
const XATTR_ENTRY_VALUE_GAP: usize = XATTR_ALIGN;

/// The xattr entry of an ext2 inode, organized on an xattr block.
#[repr(C)]
#[derive(Clone, Copy, Pod, Debug)]
struct XattrEntry {
    name_len: u8,
    name_index: u8,
    value_offset: u16,
    value_block: u32,
    value_len: u32,
    hash: u32,
}

const XATTR_ENTRY_SIZE: usize = size_of::<XattrEntry>();

/// An xattr object of an ext2 inode.
/// An xattr is used to manage special 'name-value' pairs of an inode.
///
/// An xattr block layout (objects are aligned to 4 bytes):
///  +--------------------+
///  | XattrHeader        |
///  | XattrEntry 1       | |
///  | XattrName  1       | |
///  | XattrEntry 2       | | growing downwards
///  | XattrName  2       | |
///  | XattrEntry 3       | v
///  | XattrName  3       |
///  | ...                |
///  | Gap (4 null bytes) |
///  | ...                |
///  | XattrValue 3       | ^
///  | XattrValue 2       | | growing upwards
///  | XattrValue 1       | |
///  +--------------------+
#[derive(Debug)]
pub(super) struct Xattr {
    /// The buffer of the xattr block, which always keeps its content up-to-date.
    blocks_buf: USegment,
    /// A cache to assist the xattr operations.
    cache: RwMutex<Dirty<XattrCache>>,
    inode: Weak<Inode>,
    fs: Weak<Ext2>,
}

/// Gathers helper metadata to describe an xattr block. This is
/// primarily for convenience when manipulating xattr entries or values.
/// Without this cache, the raw buffer would need to be parsed every time.
#[derive(Debug)]
struct XattrCache {
    bid: Bid,
    header: Option<XattrHeader>,
    entries: BTreeMap<usize, XattrEntry>, // K: offset, V: entry
    values: BTreeMap<usize, usize>,       // K: offset, V: len
    capacity_bytes: usize,
}

/// The number of blocks for an xattr. This value should be
/// the consistent with `XattrHeader::nblocks`.
pub(super) const XATTR_NBLOCKS: usize = 1;

impl XattrEntry {
    fn total_len(&self) -> usize {
        XATTR_ENTRY_SIZE + (self.name_len as usize).align_up(XATTR_ALIGN)
    }

    fn target_len(name_len: usize) -> usize {
        XATTR_ENTRY_SIZE + name_len.align_up(XATTR_ALIGN)
    }
}

impl Xattr {
    pub fn new(bid: Bid, inode: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        let blocks_buf: USegment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(XATTR_NBLOCKS)
            .unwrap()
            .into();
        Self {
            cache: RwMutex::new(Dirty::new(XattrCache::new(bid, blocks_buf.size()))),
            blocks_buf,
            inode,
            fs,
        }
    }

    /// Lazily initialize the xattr structures only when actual xattr operations are performed.
    fn lazy_init(&self) -> Result<()> {
        let fs = self.fs();
        let cache = self.cache.upread();

        // Need to allocate a new xattr block
        if cache.bid.to_raw() == 0 {
            assert!(cache.header.is_none());
            let new_bid = {
                let allocated = fs
                    .alloc_blocks(self.inode().block_group_idx(), XATTR_NBLOCKS as _)
                    .ok_or(Error::new(Errno::ENOSPC))?
                    .start as u64;
                Bid::new(allocated)
            };

            let new_header = XattrHeader::default();
            self.blocks_buf.write_val(0, &new_header)?;

            let mut cache = cache.upgrade();
            cache.header = Some(new_header);
            cache.bid = new_bid;
            self.inode().set_acl(new_bid);
        // Need to load the xattr block from device
        } else if cache.header.is_none() {
            fs.block_device().read_blocks(
                cache.bid,
                BioSegment::new_from_segment(self.blocks_buf.clone(), BioDirection::FromDevice),
            )?;

            let header = self.blocks_buf.read_val::<XattrHeader>(0)?;
            if header.magic != EXT2_XATTR_MAGIC {
                return_errno_with_message!(Errno::EINVAL, "invalid xattr magic");
            }

            let mut cache = cache.upgrade();
            cache.header = Some(header);
            cache.build_entries_from_buf(&self.blocks_buf)?;
        }
        Ok(())
    }

    pub fn set(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.lazy_init()?;

        let namespace = name.namespace();
        let name_len = name.full_name_len();
        let value_len = value_reader.remain();
        let cache = self.cache.upread();
        if let Some((offset, mut entry)) = cache.find_entry(&name, &self.blocks_buf) {
            if flags.contains(XattrSetFlags::CREATE_ONLY) {
                return_errno_with_message!(Errno::EEXIST, "the target xattr already exists");
            }

            if value_len <= entry.value_len as usize {
                self.blocks_buf
                    .write(entry.value_offset as _, value_reader)?;

                if value_len != entry.value_len as usize {
                    entry.value_len = value_len as _;
                    self.blocks_buf.write_val(offset, &entry)?;

                    let mut cache = cache.upgrade();
                    let _ = cache.entries.insert(offset, entry);
                    let _ = cache.values.insert(entry.value_offset as _, value_len);
                }
            } else {
                let new_value_offset = cache
                    .find_room_for_new_value(value_len)
                    .ok_or(Error::new(Errno::ENOSPC))?;
                self.blocks_buf.write(new_value_offset, value_reader)?;

                let mut cache = cache.upgrade();
                let _ = cache.values.remove(&(entry.value_offset as _));
                entry.value_offset = new_value_offset as _;
                entry.value_len = value_len as _;
                self.blocks_buf.write_val(offset, &entry)?;

                let _ = cache.entries.insert(offset, entry);
                let _ = cache.values.insert(new_value_offset as _, value_len);
            }
        } else {
            if flags.contains(XattrSetFlags::REPLACE_ONLY) {
                return_errno_with_message!(Errno::ENODATA, "the target xattr does not exist");
            }

            let new_entry_offset = cache
                .find_room_for_new_entry(name_len)
                .ok_or(Error::new(Errno::ENOSPC))?;
            let new_value_offset = cache
                .find_room_for_new_value(value_len)
                .ok_or(Error::new(Errno::ENOSPC))?;
            let new_entry = XattrEntry {
                name_len: name_len as _,
                name_index: namespace as _,
                value_offset: new_value_offset as _,
                value_block: 0, // TBD
                value_len: value_len as _,
                hash: 0, // TBD
            };

            self.blocks_buf.write_val(new_entry_offset, &new_entry)?;
            self.blocks_buf.write_bytes(
                new_entry_offset + XATTR_ENTRY_SIZE,
                name.full_name().as_bytes(),
            )?;
            self.blocks_buf.write(new_value_offset, value_reader)?;

            let mut cache = cache.upgrade();
            let _ = cache.entries.insert(new_entry_offset, new_entry);
            let _ = cache.values.insert(new_value_offset, value_len);
        }

        Ok(())
    }

    pub fn get(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.lazy_init()?;

        let value_avail_len = value_writer.avail();
        let (_, entry) = self
            .cache
            .read()
            .find_entry(&name, &self.blocks_buf)
            .ok_or(Error::new(Errno::ENODATA))?;

        let value_len = entry.value_len as usize;
        if value_avail_len == 0 {
            return Ok(value_len);
        }
        if value_len > value_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr value buffer is too small");
        }

        value_writer.write_fallible(
            self.blocks_buf
                .reader()
                .to_fallible()
                .skip(entry.value_offset as usize)
                .limit(value_len),
        )?;

        Ok(value_len)
    }

    pub fn list(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.lazy_init()?;

        let list_avail_len = list_writer.avail();
        let cache = self.cache.read();

        let target_list: Vec<_> = cache
            .entries
            .iter()
            .filter_map(|(offset, entry)| {
                if namespace.is_user() && entry.name_index != XattrNamespace::User as u8 {
                    None
                } else {
                    Some((offset, entry.name_len as usize))
                }
            })
            .collect();
        // Include the null byte following each name
        let list_actual_len = target_list
            .iter()
            .map(|(_, name_len)| name_len)
            .sum::<usize>()
            + target_list.len();

        if list_avail_len == 0 {
            return Ok(list_actual_len);
        }
        if list_actual_len > list_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr list buffer is too small");
        }

        for (offset, name_len) in target_list {
            let mut reader = self.blocks_buf.reader().to_fallible();
            let name_reader = reader.skip(offset + XATTR_ENTRY_SIZE).limit(name_len);

            list_writer.write_fallible(name_reader)?;
            list_writer.write_val(&0u8)?;
        }

        Ok(list_actual_len)
    }

    pub fn remove(&self, name: XattrName) -> Result<()> {
        self.lazy_init()?;

        let cache = self.cache.upread();
        let (offset, entry) =
            cache
                .find_entry(&name, &self.blocks_buf)
                .ok_or(Error::with_message(
                    Errno::ENODATA,
                    "the target xattr does not exist",
                ))?;

        let len = entry.total_len();
        self.blocks_buf
            .writer()
            .to_fallible()
            .skip(offset)
            .limit(len)
            .fill_zeros(len)?;

        let mut cache = cache.upgrade();
        let _ = cache.entries.remove(&offset);
        let _ = cache.values.remove(&(entry.value_offset as _));

        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        let cache = self.cache.upread();
        if cache.is_dirty() {
            self.fs().block_device().write_blocks(
                cache.bid,
                BioSegment::new_from_segment(self.blocks_buf.clone(), BioDirection::ToDevice),
            )?;
            cache.upgrade().clear_dirty();
        }
        Ok(())
    }

    pub fn free(&self) -> Result<()> {
        let cache = self.cache.upread();
        let bid = cache.bid.to_raw() as Ext2Bid;
        if bid == 0 {
            return Ok(());
        }
        self.fs().free_blocks(bid..bid)?;
        cache.upgrade().bid = Bid::new(0);
        Ok(())
    }

    fn fs(&self) -> Arc<Ext2> {
        self.fs.upgrade().unwrap()
    }

    fn inode(&self) -> Arc<Inode> {
        self.inode.upgrade().unwrap()
    }
}

impl XattrCache {
    pub fn new(bid: Bid, capacity_bytes: usize) -> Self {
        Self {
            bid,
            header: None,
            entries: BTreeMap::new(),
            values: BTreeMap::new(),
            capacity_bytes,
        }
    }

    pub fn build_entries_from_buf(&mut self, blocks_buf: &USegment) -> Result<()> {
        let mut offset = XATTR_HEADER_SIZE;

        while offset < self.capacity_bytes - XATTR_ENTRY_VALUE_GAP - XATTR_ENTRY_SIZE {
            let entry = blocks_buf.read_val::<XattrEntry>(offset)?;
            if entry.name_len == 0
                || XattrNamespace::try_from(entry.name_index).is_err()
                || entry.value_offset as usize + entry.value_len as usize > self.capacity_bytes
            {
                offset += XATTR_ALIGN;
                continue;
            }

            let _ = self.entries.insert(offset, entry);
            let _ = self
                .values
                .insert(entry.value_offset as _, entry.value_len as _);
            offset += entry.total_len();
        }
        Ok(())
    }

    pub fn find_entry(&self, name: &XattrName, buf: &USegment) -> Option<(usize, XattrEntry)> {
        let namespace = name.namespace();
        let name_bytes = name.full_name().as_bytes();
        let name_len = name_bytes.len();
        debug_assert!(name_len > 0);
        let mut name_buf = [0u8; XATTR_NAME_MAX_LEN];
        for (offset, entry) in &self.entries {
            if entry.name_index == namespace as u8 && entry.name_len == name_len as u8 {
                buf.read_bytes(offset + XATTR_ENTRY_SIZE, &mut name_buf[..name_len])
                    .unwrap();
                if &name_buf[..name_len] == name_bytes {
                    return Some((*offset, *entry));
                }
            }
        }
        None
    }

    pub fn find_room_for_new_entry(&self, name_len: usize) -> Option<usize> {
        let target_len = XattrEntry::target_len(name_len);

        // Fast path: find at the entries' end
        let bottom = self
            .values
            .first_key_value()
            .map(|(offset, _)| *offset)
            .unwrap_or(self.capacity_bytes)
            - XATTR_ENTRY_VALUE_GAP;
        let last_entry_end = self
            .entries
            .last_key_value()
            .map(|(offset, entry)| *offset + entry.total_len())
            .unwrap_or(XATTR_HEADER_SIZE);
        if bottom - last_entry_end >= target_len {
            return Some(last_entry_end);
        }

        // Slow path: find around the gaps
        let mut pre_end = XATTR_HEADER_SIZE;
        for (offset, entry) in &self.entries {
            if offset - pre_end >= target_len {
                return Some(pre_end);
            } else {
                pre_end = offset + entry.total_len();
            }
        }

        None
    }

    pub fn find_room_for_new_value(&self, value_len: usize) -> Option<usize> {
        let target_len = value_len.align_up(XATTR_ALIGN);

        // Fast path: find at the values' end
        let top = self
            .entries
            .last_key_value()
            .map(|(offset, entry)| *offset + entry.total_len())
            .unwrap_or(XATTR_HEADER_SIZE)
            + XATTR_ENTRY_VALUE_GAP;
        let last_value_start = self
            .values
            .first_key_value()
            .map(|(offset, _)| *offset)
            .unwrap_or(self.capacity_bytes);

        if last_value_start - top >= target_len {
            return Some(last_value_start - target_len);
        }

        // Slow path: find around the gaps
        let mut pre_end = self.capacity_bytes;
        for (offset, value_len) in self.values.iter().rev() {
            let cur_end = offset + value_len.align_up(XATTR_ALIGN);
            if pre_end - cur_end >= target_len {
                return Some(cur_end);
            } else {
                pre_end = *offset;
            }
        }

        None
    }
}

impl Default for XattrHeader {
    fn default() -> Self {
        Self {
            magic: EXT2_XATTR_MAGIC,
            nblocks: XATTR_NBLOCKS as _,
            ref_count: Default::default(),
            hash: Default::default(),
            reserved: Default::default(),
        }
    }
}
