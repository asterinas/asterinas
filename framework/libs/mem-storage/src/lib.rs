#![no_std]

extern crate alloc;

mod error;

use alloc::vec::Vec;
use pod::Pod;

pub use crate::error::Error;

pub type Result<T> = core::result::Result<T, self::Error>;

pub trait GenericIo: Send + Sync {
    fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<()>;

    fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<()>;

    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut val = T::new_uninit();
        self.read_bytes_at(offset, val.as_bytes_mut())?;
        Ok(val)
    }

    fn read_slice<T: Pod>(&self, offset: usize, slice: &mut [T]) -> Result<()> {
        let buf = unsafe { core::mem::transmute(slice) };
        self.read_bytes_at(offset, buf)
    }

    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.write_bytes_at(offset, new_val.as_bytes())?;
        Ok(())
    }

    fn write_slice<T: Pod>(&self, offset: usize, slice: &[T]) -> Result<()> {
        let buf = unsafe { core::mem::transmute(slice) };
        self.write_bytes_at(offset, buf)
    }
}

/// The MemStorage consists of several continuous memory areas.
pub trait MemStorage: Send + Sync {
    /// Returns an iterator to iterate all the memory areas.
    fn mem_areas(&self, is_writable: bool) -> Result<MemStorageIterator>;

    /// Returns the size of the MemStorage.
    fn total_len(&self) -> usize;

    /// Returns an iterator for the part of the memory areas.
    fn mem_areas_slice(
        &self,
        start: usize,
        len: usize,
        is_writable: bool,
    ) -> Result<MemStorageIterator> {
        assert!(self.total_len() >= start + len);

        let mut slice_vec: Vec<MemArea> = Vec::new();
        let mut start_offset = start;
        let mut remaining_len = len;

        for mem_area in self.mem_areas(is_writable)? {
            if start_offset >= mem_area.len() {
                start_offset -= mem_area.len();
                continue;
            }

            if remaining_len == 0 {
                break;
            }

            let mem_area_slice = {
                let slice_len = (mem_area.len() - start_offset).min(remaining_len);
                unsafe { mem_area.slice(start_offset, slice_len) }
            };
            slice_vec.push(mem_area_slice);

            start_offset = 0;
            remaining_len -= len;
        }

        Ok(MemStorageIterator::from_vec(slice_vec))
    }

    /// Create a slice of MemStorage, the slice is also a MemStorage.
    fn slice(&self, start: usize, len: usize) -> MemStorageSlice
    where
        Self: Sized,
    {
        assert!(self.total_len() >= start + len);

        MemStorageSlice {
            storage: self,
            start,
            len,
        }
    }

    /// Copy from another MemStorage.
    fn copy_from(&self, src: &dyn MemStorage) -> Result<()> {
        assert!(self.total_len() == src.total_len());

        let mut self_iter = self.mem_areas(true)?;
        let mut src_iter = src.mem_areas(false)?;
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

impl<T: MemStorage> MemStorage for Vec<T> {
    fn mem_areas(&self, is_writable: bool) -> Result<MemStorageIterator> {
        let mut total_vec: Vec<MemArea> = Vec::new();
        for item in self.iter() {
            let mut item_vec = item.mem_areas(is_writable)?.inner;
            total_vec.append(&mut item_vec);
        }
        Ok(MemStorageIterator::from_vec(total_vec))
    }

    fn total_len(&self) -> usize {
        self.iter().fold(0, |total, item| total + item.total_len())
    }
}

impl<T: MemStorage> MemStorage for &[T] {
    fn mem_areas(&self, is_writable: bool) -> Result<MemStorageIterator> {
        let mut total_vec: Vec<MemArea> = Vec::new();
        for item in self.iter() {
            let mut item_vec = item.mem_areas(is_writable)?.inner;
            total_vec.append(&mut item_vec);
        }
        Ok(MemStorageIterator::from_vec(total_vec))
    }

    fn total_len(&self) -> usize {
        self.iter().fold(0, |total, item| total + item.total_len())
    }
}

impl<T: MemStorage + ?Sized> GenericIo for T {
    fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        if offset + buf.len() > self.total_len() {
            return Err(Error::InvalidArgs);
        }

        let is_writable = false;
        let mut buf_offset = 0;
        for mem_area in self.mem_areas_slice(offset, buf.len(), is_writable)? {
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

    fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<()> {
        if offset + buf.len() > self.total_len() {
            return Err(Error::InvalidArgs);
        }

        let is_writable = true;
        let mut buf_offset = 0;
        for mut mem_area in self.mem_areas_slice(offset, buf.len(), is_writable)? {
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

/// An Iterator to iterate over the MemStorage.
#[derive(Debug)]
pub struct MemStorageIterator {
    inner: Vec<MemArea>,
    left_idx: usize,
    right_idx: usize,
}

impl MemStorageIterator {
    pub fn from_iterator(iter: impl Iterator<Item = MemArea>) -> Self {
        Self {
            inner: iter.collect(),
            left_idx: 0,
            right_idx: 0,
        }
    }

    pub fn from_vec(vec: Vec<MemArea>) -> Self {
        Self {
            inner: vec,
            left_idx: 0,
            right_idx: 0,
        }
    }
}

impl Iterator for MemStorageIterator {
    type Item = MemArea;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.inner.get(self.left_idx);
        if item.is_some() {
            self.left_idx += 1;
        }
        item.cloned()
    }
}

impl DoubleEndedIterator for MemStorageIterator {
    fn next_back(&mut self) -> Option<Self::Item> {
        let item = self.inner.iter().rev().nth(self.right_idx);
        if item.is_some() {
            self.right_idx += 1;
        }
        item.cloned()
    }
}

#[derive(Clone, Debug)]
pub struct MemArea(MemAreaInner);

#[derive(Clone, Debug)]
enum MemAreaInner {
    Ref(*const u8, usize),
    Mut(*mut u8, usize),
}

impl MemArea {
    pub unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Self {
        Self(MemAreaInner::Ref(ptr, len))
    }

    pub unsafe fn from_raw_parts_mut(ptr: *mut u8, len: usize) -> Self {
        Self(MemAreaInner::Mut(ptr, len))
    }

    pub unsafe fn slice(&self, start: usize, len: usize) -> Self {
        debug_assert!(start + len <= self.len());

        match &self.0 {
            MemAreaInner::Ref(ptr, _) => Self::from_raw_parts((*ptr).add(start), len),
            MemAreaInner::Mut(ptr, _) => Self::from_raw_parts_mut((*ptr).add(start), len),
        }
    }

    pub fn from_slice(slice: &[u8]) -> Self {
        Self(MemAreaInner::Ref(slice.as_ptr(), slice.len()))
    }

    pub fn from_slice_mut(slice: &mut [u8]) -> Self {
        Self(MemAreaInner::Mut(slice.as_mut_ptr(), slice.len()))
    }

    /// Returns a pointer to the slice’s buffer.
    pub fn as_ptr(&self) -> *const u8 {
        match &self.0 {
            MemAreaInner::Ref(ptr, _) => *ptr,
            MemAreaInner::Mut(ptr, _) => *ptr as _,
        }
    }

    /// Returns a mutable pointer to the slice’s buffer.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        match &self.0 {
            MemAreaInner::Ref(_, _) => panic!(),
            MemAreaInner::Mut(ptr, _) => *ptr,
        }
    }

    /// Returns a slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.as_ptr() as _, self.len()) }
    }

    /// Returns a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.as_mut_ptr(), self.len()) }
    }

    /// Returns the length.
    pub fn len(&self) -> usize {
        match &self.0 {
            MemAreaInner::Ref(_, len) => *len,
            MemAreaInner::Mut(_, len) => *len,
        }
    }

    /// Returns if length is zero.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
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

    fn mem_areas(&self, is_writable: bool) -> Result<MemStorageIterator> {
        self.storage
            .mem_areas_slice(self.start, self.len, is_writable)
    }
}
