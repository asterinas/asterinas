// SPDX-License-Identifier: MPL-2.0

use ostd::mm::UntypedMem;

use super::{prelude::*, Ext2};
use crate::fs::utils::{XattrFlags, XattrNamespace, XATTR_NAME_MAX_LEN};

const EXT2_XATTR_MAGIC: u32 = 0xEA020000;

///  +------------------+
///  | header           |
///  | entry 1          | |
///  | entry 2          | | growing downwards
///  | entry 3          | v
///  | four null bytes  |
///  | . . .            |
///  | value 1          | ^
///  | value 3          | | growing upwards
///  | value 2          | |
///  +------------------+
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

impl XattrEntry {
    fn total_len(&self) -> usize {
        XATTR_ENTRY_SIZE + (self.name_len as usize).align_up(XATTR_ALIGN)
    }

    fn target_len(name_len: usize) -> usize {
        XATTR_ENTRY_SIZE + name_len.align_up(XATTR_ALIGN)
    }
}

const XATTR_ENTRY_SIZE: usize = size_of::<XattrEntry>();

pub(super) struct Xattr {
    blocks: USegment,
    bid: Bid,
    cache: RwLock<Dirty<XattrCache>>,
    fs: Weak<Ext2>,
}

#[derive(Debug)]
struct XattrCache {
    entries: BTreeMap<usize, XattrEntry>,
    values: BTreeMap<usize, usize>,
    capacity_bytes: usize,
}

impl XattrCache {
    fn new(capacity_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            values: BTreeMap::new(),
            capacity_bytes,
        }
    }

    fn build(&mut self, blocks: &USegment) -> Result<()> {
        let mut offset = XATTR_HEADER_SIZE;

        while offset < self.capacity_bytes - XATTR_ENTRY_VALUE_GAP - XATTR_ENTRY_SIZE {
            let entry = blocks.read_val::<XattrEntry>(offset)?;
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

    fn find_entry(
        &self,
        namespace: XattrNamespace,
        name: &str,
        buf: &USegment,
    ) -> Option<(usize, XattrEntry)> {
        let name_len = name.len();
        debug_assert!(name_len > 0);
        let mut name_buf = [0u8; XATTR_NAME_MAX_LEN];
        for (offset, entry) in &self.entries {
            if entry.name_index == namespace as u8 && entry.name_len == name_len as u8 {
                buf.read_bytes(offset + XATTR_ENTRY_SIZE, &mut name_buf[..name_len])
                    .unwrap();
                if &name_buf[..name_len] == name.as_bytes() {
                    return Some((*offset, *entry));
                }
            }
        }
        None
    }

    fn find_room_for_new_entry(&self, name_len: usize) -> Option<usize> {
        let target_len = XattrEntry::target_len(name_len);

        // Fast path
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
        if last_entry_end - bottom >= target_len {
            return Some(last_entry_end);
        }

        // Slow path
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

    fn find_room_for_new_value(&self, value_len: usize) -> Option<usize> {
        let target_len = value_len.align_up(XATTR_ALIGN);

        // Fast path
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

        // Slow path
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

impl Xattr {
    pub fn load(bid: Bid, fs: Weak<Ext2>) -> Result<Self> {
        let blocks: USegment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)?
            .into();
        fs.upgrade().unwrap().block_device().read_blocks(
            bid,
            BioSegment::new_from_segment(blocks.clone(), BioDirection::FromDevice),
        )?;

        let header = blocks.read_val::<XattrHeader>(0)?;
        if header.magic == 0 {
            return Ok(Self::new(blocks, bid, fs));
        }
        if header.magic != EXT2_XATTR_MAGIC {
            return_errno_with_message!(Errno::EINVAL, "invalid xattr magic");
        }

        let mut cache = XattrCache::new(blocks.size());
        cache.build(&blocks)?;
        Ok(Self {
            blocks,
            bid,
            cache: RwLock::new(Dirty::new(cache)),
            fs,
        })
    }

    fn new(blocks: USegment, bid: Bid, fs: Weak<Ext2>) -> Self {
        let cache = RwLock::new(Dirty::new(XattrCache::new(blocks.size())));
        Self {
            blocks,
            bid,
            cache,
            fs,
        }
    }

    pub fn set(
        &self,
        namespace: XattrNamespace,
        name: &str,
        value_reader: &mut VmReader,
        flags: XattrFlags,
    ) -> Result<()> {
        let name_len = name.len();
        let value_len = value_reader.remain();
        let cache = self.cache.upread();
        if let Some((offset, mut entry)) = cache.find_entry(namespace, name, &self.blocks) {
            if flags.contains(XattrFlags::XATTR_CREATE) {
                return_errno_with_message!(Errno::EEXIST, "the target xattr already exists");
            }

            if value_len <= entry.value_len as usize {
                self.blocks.write(entry.value_offset as _, value_reader)?;

                if value_len != entry.value_len as usize {
                    entry.value_len = value_len as _;
                    self.blocks.write_val(offset, &entry)?;

                    let mut cache = cache.upgrade();
                    let _ = cache.entries.insert(offset, entry);
                    let _ = cache.values.insert(entry.value_offset as _, value_len);
                }
            } else {
                let new_value_offset = cache
                    .find_room_for_new_value(value_len)
                    .ok_or(Error::new(Errno::ENOSPC))?;
                self.blocks.write(new_value_offset, value_reader)?;

                let mut cache = cache.upgrade();
                let _ = cache.values.remove(&(entry.value_offset as _));
                entry.value_offset = new_value_offset as _;
                entry.value_len = value_len as _;
                self.blocks.write_val(offset, &entry)?;

                let _ = cache.entries.insert(offset, entry);
                let _ = cache.values.insert(new_value_offset as _, value_len);
            }
        } else {
            if flags.contains(XattrFlags::XATTR_REPLACE) {
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

            self.blocks.write_val(new_entry_offset, &new_entry)?;
            self.blocks
                .write_bytes(new_entry_offset + XATTR_ENTRY_SIZE, name.as_bytes())?;
            self.blocks.write(new_value_offset, value_reader)?;

            let mut cache = cache.upgrade();
            let _ = cache.entries.insert(new_entry_offset, new_entry);
            let _ = cache.values.insert(new_value_offset, value_len);
        }
        Ok(())
    }

    pub fn get(
        &self,
        namespace: XattrNamespace,
        name: &str,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        let value_avail_len = value_writer.avail();
        let (_, entry) = self
            .cache
            .read()
            .find_entry(namespace, name, &self.blocks)
            .ok_or(Error::new(Errno::ENODATA))?;

        let value_len = entry.value_len as usize;
        if value_avail_len == 0 {
            return Ok(value_len);
        }
        if value_len > value_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr value buffer is too small");
        }

        value_writer.write_fallible(
            &mut self
                .blocks
                .reader()
                .to_fallible()
                .skip(entry.value_offset as usize)
                .limit(value_len),
        )?;
        Ok(value_len)
    }

    pub fn list(&self, list_writer: &mut VmWriter) -> Result<usize> {
        let list_avail_len = list_writer.avail();
        let cache = self.cache.read();
        let list_total_len = cache.entries.len()
            + cache
                .entries
                .values()
                .map(|entry| entry.name_len as usize)
                .sum::<usize>();

        if list_avail_len == 0 {
            return Ok(list_total_len);
        }
        if list_total_len > list_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr list buffer is too small");
        }

        for (offset, entry) in &cache.entries {
            // TODO: Check namespace
            let mut name_reader = self
                .blocks
                .reader()
                .to_fallible()
                .skip(offset + XATTR_ENTRY_SIZE)
                .limit(entry.name_len as usize);
            list_writer.write_fallible(&mut name_reader)?;
            list_writer.write_val(&0u8)?;
        }
        Ok(list_total_len)
    }

    pub fn remove(&self, namespace: XattrNamespace, name: &str) -> Result<()> {
        let cache = self.cache.upread();

        let (offset, entry) =
            cache
                .find_entry(namespace, name, &self.blocks)
                .ok_or(Error::with_message(
                    Errno::ENODATA,
                    "the target xattr does not exist",
                ))?;

        let len = entry.total_len();
        self.blocks
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

    pub fn persist(&self) -> Result<()> {
        if self.cache.read().is_dirty() {
            self.fs.upgrade().unwrap().block_device().write_blocks(
                self.bid,
                BioSegment::new_from_segment(self.blocks.clone(), BioDirection::ToDevice),
            )?;
        }
        Ok(())
    }
}
