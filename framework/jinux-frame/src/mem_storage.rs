use crate::{prelude::*, vm::VmIo, Error};

use core::marker::PhantomData;

/// The MemStorage consists of several continuous memory areas.
pub trait MemStorage: Send + Sync {
    /// Returns an iterator to iterate all the continuous memory areas.
    fn mem_areas(&self, is_writable: bool) -> MemStorageIterator;

    /// Returns the total length of memory.
    fn total_len(&self) -> usize {
        self.mem_areas(false)
            .fold(0, |total, mem_area| total + mem_area.len())
    }

    /// Returns an iterator for the part of the memory areas.
    ///
    /// Here we provide a default implementation based on the `mem_areas` method,
    /// and it may be slow because `mem_areas` may access the entire data.
    /// One can re-implement it by accessing a part of the data.
    fn mem_areas_slice(&self, start: usize, len: usize, is_writable: bool) -> MemStorageIterator {
        debug_assert!(start < self.total_len());
        debug_assert!(start + len <= self.total_len());

        let mut slice_vec: Vec<MemArea> = Vec::new();
        let mut start_offset = start;
        let mut remaining_len = len;

        for mem_area in self.mem_areas(is_writable) {
            if start_offset >= mem_area.len() {
                start_offset -= mem_area.len();
                continue;
            }

            if remaining_len == 0 {
                break;
            }

            let slice_len = (mem_area.len() - start_offset).min(remaining_len);
            let mem_area_slice = mem_area.slice(start_offset, slice_len);
            slice_vec.push(mem_area_slice);

            start_offset = 0;
            remaining_len -= slice_len;
        }

        MemStorageIterator::from_vec(slice_vec)
    }
}

impl dyn MemStorage {
    /// Creates a slice of MemStorage, the slice is also a MemStorage.
    pub fn slice(&self, start: usize, len: usize) -> MemStorageSlice {
        debug_assert!(start < self.total_len());
        debug_assert!(start + len <= self.total_len());

        MemStorageSlice {
            storage: self,
            start,
            len,
        }
    }

    /// Copies the data from `src` into `self`.
    ///
    /// The length of `src` must be the same as `self`.
    pub fn copy_from(&self, src: &dyn MemStorage) -> Result<()> {
        debug_assert!(self.total_len() == src.total_len());

        let mut self_iter = self.mem_areas(true);
        let mut src_iter = src.mem_areas(false);
        let Some(mut src_mem_area) = src_iter.next() else {
            return Ok(());
        };
        let Some(mut self_mem_area) = self_iter.next() else {
            return Ok(());
        };
        let mut self_offset = 0;
        let mut src_offset = 0;

        // Do copy in one loop
        loop {
            let copy_len = self_mem_area.len().min(src_mem_area.len());
            unsafe {
                core::ptr::copy(
                    src_mem_area.as_ptr().add(src_offset),
                    self_mem_area.as_mut_ptr().add(self_offset),
                    copy_len,
                );
            }
            self_offset += copy_len;
            src_offset += copy_len;

            if self_mem_area.len() == self_offset {
                (self_mem_area, self_offset) = {
                    let Some(self_mem_area) = self_iter.next() else {
                        break;
                    };
                    (self_mem_area, 0)
                };
            } else {
                (src_mem_area, src_offset) = {
                    let Some(src_mem_area) = src_iter.next() else {
                        break;
                    };
                    (src_mem_area, 0)
                };
            }
        }

        Ok(())
    }
}

impl VmIo for dyn MemStorage {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        if offset + buf.len() > self.total_len() {
            return Err(Error::InvalidArgs);
        }

        if offset == self.total_len() && buf.is_empty() {
            return Ok(());
        }

        let mut buf_offset = 0;
        for mem_area in self.mem_areas_slice(offset, buf.len(), false) {
            if buf_offset == buf.len() {
                break;
            }

            let buf_remaining = &mut buf[buf_offset..];
            let copy_len = mem_area.len().min(buf_remaining.len());
            unsafe {
                core::ptr::copy(mem_area.as_ptr(), buf_remaining.as_mut_ptr(), copy_len);
            }
            buf_offset += copy_len;
        }

        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        if offset + buf.len() > self.total_len() {
            return Err(Error::InvalidArgs);
        }

        if offset == self.total_len() && buf.is_empty() {
            return Ok(());
        }

        let mut buf_offset = 0;
        for mut mem_area in self.mem_areas_slice(offset, buf.len(), true) {
            if buf_offset == buf.len() {
                break;
            }

            let buf_remaining = &buf[buf_offset..];
            let copy_len = mem_area.len().min(buf_remaining.len());
            unsafe {
                core::ptr::copy(buf_remaining.as_ptr(), mem_area.as_mut_ptr(), copy_len);
            }
            buf_offset += copy_len;
        }

        Ok(())
    }
}

impl<T: MemStorage> MemStorage for Vec<T> {
    fn mem_areas(&self, is_writable: bool) -> MemStorageIterator {
        let mut total_vec: Vec<MemArea> = Vec::new();
        for item in self.iter() {
            let mut item_vec = item.mem_areas(is_writable).inner;
            total_vec.append(&mut item_vec);
        }
        MemStorageIterator::from_vec(total_vec)
    }

    fn total_len(&self) -> usize {
        self.iter().fold(0, |total, item| total + item.total_len())
    }
}

impl<T: MemStorage> MemStorage for &[T] {
    fn mem_areas(&self, is_writable: bool) -> MemStorageIterator {
        let mut total_vec: Vec<MemArea> = Vec::new();
        for item in self.iter() {
            let mut item_vec = item.mem_areas(is_writable).inner;
            total_vec.append(&mut item_vec);
        }
        MemStorageIterator::from_vec(total_vec)
    }

    fn total_len(&self) -> usize {
        self.iter().fold(0, |total, item| total + item.total_len())
    }
}

/// A continuous memory areas iterator to iterate over the MemStorage.
#[derive(Debug)]
pub struct MemStorageIterator<'a> {
    inner: Vec<MemArea<'a>>,
    left_idx: usize,
    right_idx: usize,
}

impl<'a> MemStorageIterator<'a> {
    pub fn from_iterator(iter: impl Iterator<Item = MemArea<'a>>) -> Self {
        Self {
            inner: iter.collect(),
            left_idx: 0,
            right_idx: 0,
        }
    }

    pub fn from_vec(vec: Vec<MemArea<'a>>) -> Self {
        Self {
            inner: vec,
            left_idx: 0,
            right_idx: 0,
        }
    }
}

impl<'a> Iterator for MemStorageIterator<'a> {
    type Item = MemArea<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.inner.get(self.left_idx);
        if item.is_some() {
            self.left_idx += 1;
        }
        item.cloned()
    }
}

impl DoubleEndedIterator for MemStorageIterator<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let item = self.inner.iter().rev().nth(self.right_idx);
        if item.is_some() {
            self.right_idx += 1;
        }
        item.cloned()
    }
}

/// A continuous memory area.
///
/// It is a reference to the data segment in MemStorage.
#[derive(Clone, Debug)]
pub struct MemArea<'a> {
    raw_ptr: RawPtr,
    len: usize,
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> MemArea<'a> {
    /// # Safety
    ///
    /// User must ensure the range is valid.
    pub unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Self {
        MemArea {
            raw_ptr: RawPtr::Ref(ptr),
            len,
            phantom: PhantomData,
        }
    }

    /// # Safety
    ///
    /// User must ensure the range is valid.
    pub unsafe fn from_raw_parts_mut(ptr: *mut u8, len: usize) -> Self {
        MemArea {
            raw_ptr: RawPtr::Mut(ptr),
            len,
            phantom: PhantomData,
        }
    }

    pub fn from_slice(slice: &'a [u8]) -> Self {
        MemArea {
            raw_ptr: RawPtr::Ref(slice.as_ptr()),
            len: slice.len(),
            phantom: PhantomData,
        }
    }

    pub fn from_slice_mut(slice: &'a mut [u8]) -> Self {
        MemArea {
            raw_ptr: RawPtr::Mut(slice.as_mut_ptr()),
            len: slice.len(),
            phantom: PhantomData,
        }
    }

    /// Returns a slice of the memory.
    pub fn slice(&self, start: usize, len: usize) -> Self {
        debug_assert!(start < self.len());
        debug_assert!(start + len <= self.len());

        MemArea {
            raw_ptr: unsafe {
                match self.raw_ptr {
                    RawPtr::Ref(ptr) => RawPtr::Ref(ptr.add(start)),
                    RawPtr::Mut(ptr) => RawPtr::Mut(ptr.add(start)),
                }
            },
            len,
            phantom: self.phantom,
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        match self.raw_ptr {
            RawPtr::Ref(ptr) => ptr,
            RawPtr::Mut(ptr) => ptr as _,
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        match self.raw_ptr {
            RawPtr::Ref(ptr) => panic!(),
            RawPtr::Mut(ptr) => ptr,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, Debug)]
enum RawPtr {
    Ref(*const u8),
    Mut(*mut u8),
}

/// A slice of the MemStorage, it is also a MemStorage.
pub struct MemStorageSlice<'a> {
    storage: &'a dyn MemStorage,
    start: usize,
    len: usize,
}

impl<'a> MemStorage for MemStorageSlice<'a> {
    fn total_len(&self) -> usize {
        self.len
    }

    fn mem_areas(&self, is_writable: bool) -> MemStorageIterator {
        self.storage
            .mem_areas_slice(self.start, self.len, is_writable)
    }
}
