// SPDX-License-Identifier: MPL-2.0

//! A contiguous range of pages.

use core::{mem::ManuallyDrop, ops::Range};

use super::{inc_page_ref_count, meta::FrameMeta, Frame};
use crate::mm::{Paddr, UFrameMeta, PAGE_SIZE};

/// A contiguous range of homogeneous physical memory pages.
///
/// This is a handle to many contiguous pages. It will be more lightweight
/// than owning an array of page handles.
///
/// The ownership is achieved by the reference counting mechanism of pages.
/// When constructing a `Segment`, the page handles are created then
/// forgotten, leaving the reference count. When dropping a it, the page
/// handles are restored and dropped, decrementing the reference count.
///
/// All the metadata of the pages are homogeneous, i.e., they are of the same
/// type.
#[derive(Debug)]
#[repr(transparent)]
pub struct Segment<M: FrameMeta + ?Sized> {
    range: Range<Paddr>,
    _marker: core::marker::PhantomData<M>,
}

/// A contiguous range of homogeneous physical memory frames that have any metadata.
///
/// In other words, the metadata of the frames are of the same type but the type
/// is not known at compile time. An [`DynSegment`] as a parameter accepts any
/// type of segments.
///
/// The usage of this frame will not be changed while this object is alive.
pub type DynSegment = Segment<dyn FrameMeta>;

/// A contiguous range of homogeneous untyped physical memory pages that have any metadata.
///
/// In other words, the metadata of the frames are of the same type, and they
/// are untyped, but the type of metadata is not known at compile time. An
/// [`DynUSegment`] as a parameter accepts any untyped segments.
///
/// The usage of this frame will not be changed while this object is alive.
pub type DynUSegment = Segment<dyn UFrameMeta>;

impl<M: FrameMeta + ?Sized> Drop for Segment<M> {
    fn drop(&mut self) {
        for paddr in self.range.clone().step_by(PAGE_SIZE) {
            // SAFETY: for each page there would be a forgotten handle
            // when creating the `Segment` object.
            drop(unsafe { Frame::<M>::from_raw(paddr) });
        }
    }
}

impl<M: FrameMeta + ?Sized> Clone for Segment<M> {
    fn clone(&self) -> Self {
        for paddr in self.range.clone().step_by(PAGE_SIZE) {
            // SAFETY: for each page there would be a forgotten handle
            // when creating the `Segment` object, so we already have
            // reference counts for the pages.
            unsafe { inc_page_ref_count(paddr) };
        }
        Self {
            range: self.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: FrameMeta> Segment<M> {
    /// Creates a new `Segment` from unused pages.
    ///
    /// The caller must provide a closure to initialize metadata for all the pages.
    /// The closure receives the physical address of the page and returns the
    /// metadata, which is similar to [`core::array::from_fn`].
    ///
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is invalid or not aligned;
    ///  - any of the pages are already in use.
    pub fn from_unused<F>(range: Range<Paddr>, mut metadata_fn: F) -> Self
    where
        F: FnMut(Paddr) -> M,
    {
        for paddr in range.clone().step_by(PAGE_SIZE) {
            let _ = ManuallyDrop::new(Frame::<M>::from_unused(paddr, metadata_fn(paddr)));
        }
        Self {
            range,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: FrameMeta + ?Sized> Segment<M> {
    /// Gets the start physical address of the contiguous pages.
    pub fn start_paddr(&self) -> Paddr {
        self.range.start
    }

    /// Gets the end physical address of the contiguous pages.
    pub fn end_paddr(&self) -> Paddr {
        self.range.end
    }

    /// Gets the length in bytes of the contiguous pages.
    pub fn size(&self) -> usize {
        self.range.end - self.range.start
    }

    /// Splits the pages into two at the given byte offset from the start.
    ///
    /// The resulting pages cannot be empty. So the offset cannot be neither
    /// zero nor the length of the pages.
    ///
    /// # Panics
    ///
    /// The function panics if the offset is out of bounds, at either ends, or
    /// not base-page-aligned.
    pub fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset % PAGE_SIZE == 0);
        assert!(0 < offset && offset < self.size());

        let old = ManuallyDrop::new(self);
        let at = old.range.start + offset;

        (
            Self {
                range: old.range.start..at,
                _marker: core::marker::PhantomData,
            },
            Self {
                range: at..old.range.end,
                _marker: core::marker::PhantomData,
            },
        )
    }

    /// Gets an extra handle to the pages in the byte offset range.
    ///
    /// The sliced byte offset range in indexed by the offset from the start of
    /// the contiguous pages. The resulting pages holds extra reference counts.
    ///
    /// # Panics
    ///
    /// The function panics if the byte offset range is out of bounds, or if
    /// any of the ends of the byte offset range is not base-page aligned.
    pub fn slice(&self, range: &Range<usize>) -> Self {
        assert!(range.start % PAGE_SIZE == 0 && range.end % PAGE_SIZE == 0);
        let start = self.range.start + range.start;
        let end = self.range.start + range.end;
        assert!(start <= end && end <= self.range.end);

        for paddr in (start..end).step_by(PAGE_SIZE) {
            // SAFETY: We already have reference counts for the pages since
            // for each page there would be a forgotten handle when creating
            // the `Segment` object.
            unsafe { inc_page_ref_count(paddr) };
        }

        Self {
            range: start..end,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: FrameMeta + ?Sized> From<Frame<M>> for Segment<M> {
    fn from(page: Frame<M>) -> Self {
        let pa = page.start_paddr();
        let _ = ManuallyDrop::new(page);
        Self {
            range: pa..pa + PAGE_SIZE,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: FrameMeta + ?Sized> Iterator for Segment<M> {
    type Item = Frame<M>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.start < self.range.end {
            // SAFETY: each page in the range would be a handle forgotten
            // when creating the `Segment` object.
            let page = unsafe { Frame::<M>::from_raw(self.range.start) };
            self.range.start += PAGE_SIZE;
            // The end cannot be non-page-aligned.
            debug_assert!(self.range.start <= self.range.end);
            Some(page)
        } else {
            None
        }
    }
}

impl<M: FrameMeta> From<Segment<M>> for DynSegment {
    fn from(seg: Segment<M>) -> Self {
        let seg = ManuallyDrop::new(seg);
        Self {
            range: seg.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: FrameMeta> TryFrom<DynSegment> for Segment<M> {
    type Error = DynSegment;

    fn try_from(seg: DynSegment) -> core::result::Result<Self, Self::Error> {
        // SAFETY: for each page there would be a forgotten handle
        // when creating the `Segment` object.
        let first_frame = unsafe { Frame::<dyn FrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !(first_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            return Err(seg);
        }
        // Since segments are homogeneous, we can safely assume that the rest
        // of the frames are of the same type. We just debug-check here.
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn FrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!((frame.dyn_meta() as &dyn core::any::Any).is::<M>());
            }
        }
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        Ok(unsafe { core::mem::transmute::<DynSegment, Segment<M>>(seg) })
    }
}

impl<M: UFrameMeta> From<Segment<M>> for DynUSegment {
    fn from(seg: Segment<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(seg) }
    }
}

impl<M: UFrameMeta> From<&Segment<M>> for &DynUSegment {
    fn from(seg: &Segment<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(seg) }
    }
}

impl TryFrom<DynSegment> for DynUSegment {
    type Error = DynSegment;

    /// Try converting a [`DynSegment`] into [`DynUSegment`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(seg: DynSegment) -> core::result::Result<Self, Self::Error> {
        // SAFETY: for each page there would be a forgotten handle
        // when creating the `Segment` object.
        let first_frame = unsafe { Frame::<dyn FrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !first_frame.dyn_meta().is_untyped() {
            return Err(seg);
        }
        // Since segments are homogeneous, we can safely assume that the rest
        // of the frames are of the same type. We just debug-check here.
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn FrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!(frame.dyn_meta().is_untyped());
            }
        }
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        Ok(unsafe { core::mem::transmute::<DynSegment, DynUSegment>(seg) })
    }
}
