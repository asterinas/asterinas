// SPDX-License-Identifier: MPL-2.0

//! Manages a VM's guest-memory mappings and backing frames.

use core::{cmp, ops::Range};

use ostd::{
    mm::{HasPaddr, PageProperty, UFrame, io::util::HasVmReaderWriter}, 
    vm::{GuestPhysMemSpace, Gpaddr}
};

use crate::prelude::*;

struct MemorySlot {
    guest_start: Gpaddr,
    guest_end: Gpaddr,
    frames: Vec<UFrame>,
}

impl MemorySlot {
    fn new(guest_start: Gpaddr, memory_size: usize, frames: Vec<UFrame>) -> Result<Self> {
        let guest_end = guest_start
            .checked_add(memory_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        if frames.len().checked_mul(PAGE_SIZE) != Some(memory_size) {
            return_errno_with_message!(Errno::EINVAL, "guest memory frame count is invalid");
        }

        Ok(Self {
            guest_start,
            guest_end,
            frames,
        })
    }

    fn guest_range(&self) -> Range<Gpaddr> {
        self.guest_start..self.guest_end
    }

    fn overlaps_guest_range(&self, guest_range: &Range<Gpaddr>) -> bool {
        self.guest_start < guest_range.end && guest_range.start < self.guest_end
    }

    fn backing_frame(&self, gpa: Gpaddr) -> Option<(UFrame, usize, usize)> {
        if !(self.guest_start..self.guest_end).contains(&gpa) {
            return None;
        }

        let offset = gpa - self.guest_start;
        let frame_index = offset / PAGE_SIZE;
        let frame_offset = offset % PAGE_SIZE;
        let len = cmp::min(PAGE_SIZE - frame_offset, self.guest_end - gpa);
        Some((self.frames[frame_index].clone(), frame_offset, len))
    }
}

/// A VM's guest physical memory and its userspace-backed mappings.
pub struct VmMemory {
    guest_mem: GuestPhysMemSpace,
    slots: Mutex<BTreeMap<u32, MemorySlot>>,
}

impl VmMemory {
    pub fn new() -> Result<Self> {
        Ok(Self {
            guest_mem: GuestPhysMemSpace::new()?,
            slots: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn guest_mem(&self) -> &GuestPhysMemSpace {
        &self.guest_mem
    }

    pub fn set_region(
        &self,
        slot: u32,
        guest_start: Gpaddr,
        memory_size: usize,
        frames: Vec<UFrame>,
        prop: PageProperty,
    ) -> Result<()> {
        if memory_size == 0 {
            let old_slot = self.slots.lock().remove(&slot);
            if let Some(old_slot) = old_slot {
                self.unmap_guest_memory(old_slot.guest_range())?;
            }
            return Ok(());
        }

        let new_slot = MemorySlot::new(guest_start, memory_size, frames)?;
        let new_guest_range = new_slot.guest_range();

        let mut slots = self.slots.lock();
        for (&existing_slot_id, existing_slot) in slots.iter() {
            if existing_slot_id != slot && existing_slot.overlaps_guest_range(&new_guest_range) {
                return_errno_with_message!(Errno::EINVAL, "guest memory slots overlap");
            }
        }

        let old_slot = slots.remove(&slot);
        if let Some(old_slot) = old_slot {
            self.unmap_guest_memory(old_slot.guest_range())?;
        }
        self.map_guest_memory(new_guest_range, &new_slot.frames, prop)?;
        slots.insert(slot, new_slot);
        Ok(())
    }

    pub fn read_bytes(&self, gpa: Gpaddr, bytes: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(&mut *bytes).to_fallible();
        let read_len = self.read(gpa, &mut writer)?;
        if read_len != bytes.len() {
            return_errno_with_message!(Errno::EFAULT, "guest memory read was incomplete");
        }
        Ok(())
    }

    pub fn read_val<T: Pod>(&self, gpa: Gpaddr) -> Result<T> {
        let mut value = T::new_zeroed();
        let mut writer = VmWriter::from(value.as_mut_bytes()).to_fallible();
        let read_len = self.read(gpa, &mut writer)?;
        if read_len != size_of::<T>() {
            return_errno_with_message!(Errno::EFAULT, "guest memory read was incomplete");
        }
        Ok(value)
    }

    pub fn write_val<T: Pod>(&self, gpa: Gpaddr, value: &T) -> Result<()> {
        let mut reader = VmReader::from(value.as_bytes()).to_fallible();
        let written_len = self.write(gpa, &mut reader)?;
        if written_len != size_of::<T>() {
            return_errno_with_message!(Errno::EFAULT, "guest memory write was incomplete");
        }
        Ok(())
    }

    pub fn write_bytes(&self, gpa: Gpaddr, bytes: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(bytes).to_fallible();
        let written_len = self.write(gpa, &mut reader)?;
        if written_len != bytes.len() {
            return_errno_with_message!(Errno::EFAULT, "guest memory write was incomplete");
        }
        Ok(())
    }

    fn map_guest_memory(
        &self,
        guest_range: Range<Gpaddr>,
        frames: &[UFrame],
        prop: PageProperty,
    ) -> Result<()> {
        for i in 0.. frames.len(){
            let frame = &frames[i];
            self.guest_mem.map(guest_range.start + PAGE_SIZE * i, frame.paddr(), PAGE_SIZE, prop)?;
        }
        Ok(())
    }

    fn unmap_guest_memory(&self, guest_range: Range<Gpaddr>) -> Result<usize> {
        Ok(self.guest_mem.unmap(guest_range.start, guest_range.end)?)
    }

    fn read(&self, gpa: Gpaddr, writer: &mut VmWriter) -> Result<usize> {
        let mut current_gpa = gpa;
        let mut total_read = 0;

        while writer.avail() > 0 {
            let (frame, frame_offset, max_len) = self.backing_frame(current_gpa)?;
            let read_len = cmp::min(writer.avail(), max_len);
            let mut frame_reader = frame.reader();
            frame_reader.skip(frame_offset).limit(read_len);

            let copied = frame_reader.read_fallible(writer).map_err(Error::from)?;
            total_read += copied;
            current_gpa = current_gpa
                .checked_add(copied)
                .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        }

        Ok(total_read)
    }

    fn write(&self, gpa: Gpaddr, reader: &mut VmReader) -> Result<usize> {
        let mut current_gpa = gpa;
        let mut total_written = 0;

        while reader.remain() > 0 {
            let (frame, frame_offset, max_len) = self.backing_frame(current_gpa)?;
            let write_len = cmp::min(reader.remain(), max_len);
            let mut frame_writer = frame.writer();
            frame_writer.skip(frame_offset).limit(write_len);

            let copied = frame_writer.write_fallible(reader).map_err(Error::from)?;
            total_written += copied;
            current_gpa = current_gpa
                .checked_add(copied)
                .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        }

        Ok(total_written)
    }

    fn backing_frame(&self, gpa: Gpaddr) -> Result<(UFrame, usize, usize)> {
        let slots = self.slots.lock();
        for memory_slot in slots.values() {
            if let Some(backing_frame) = memory_slot.backing_frame(gpa) {
                return Ok(backing_frame);
            }
        }

        return_errno_with_message!(Errno::EFAULT, "guest physical address is not backed");
    }
}
