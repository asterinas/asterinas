// SPDX-License-Identifier: MPL-2.0

//! A contiguous range of frames.

use core::{fmt::Debug, mem::ManuallyDrop, ops::Range};

use super::{
    inc_frame_ref_count,
    meta::{AnyFrameMeta, GetFrameError},
    Frame,
};
use crate::mm::{AnyUFrameMeta, Paddr, PAGE_SIZE};

/// A contiguous range of homogeneous physical memory frames.
///
/// This is a handle to multiple contiguous frames. It will be more lightweight
/// than owning an array of frame handles.
///
/// The ownership is achieved by the reference counting mechanism of frames.
/// When constructing a [`Segment`], the frame handles are created then
/// forgotten, leaving the reference count. When dropping a it, the frame
/// handles are restored and dropped, decrementing the reference count.
///
/// All the metadata of the frames are homogeneous, i.e., they are of the same
/// type.
#[repr(transparent)]
pub struct Segment<M: AnyFrameMeta + ?Sized> {
    range: Range<Paddr>,
    _marker: core::marker::PhantomData<M>,
}

impl<M: AnyFrameMeta + ?Sized> Debug for Segment<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Segment({:#x}..{:#x})", self.range.start, self.range.end)
    }
}

/// A contiguous range of homogeneous untyped physical memory frames that have any metadata.
///
/// In other words, the metadata of the frames are of the same type, and they
/// are untyped, but the type of metadata is not known at compile time. An
/// [`USegment`] as a parameter accepts any untyped segments.
///
/// The usage of this frame will not be changed while this object is alive.
pub type USegment = Segment<dyn AnyUFrameMeta>;

impl<M: AnyFrameMeta + ?Sized> Drop for Segment<M> {
    fn drop(&mut self) {
        for paddr in self.range.clone().step_by(PAGE_SIZE) {
            // SAFETY: for each frame there would be a forgotten handle
            // when creating the `Segment` object.
            drop(unsafe { Frame::<M>::from_raw(paddr) });
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Clone for Segment<M> {
    fn clone(&self) -> Self {
        for paddr in self.range.clone().step_by(PAGE_SIZE) {
            // SAFETY: for each frame there would be a forgotten handle
            // when creating the `Segment` object, so we already have
            // reference counts for the frames.
            unsafe { inc_frame_ref_count(paddr) };
        }
        Self {
            range: self.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta> Segment<M> {
    /// Creates a new [`Segment`] from unused frames.
    ///
    /// The caller must provide a closure to initialize metadata for all the frames.
    /// The closure receives the physical address of the frame and returns the
    /// metadata, which is similar to [`core::array::from_fn`].
    ///
    /// It returns an error if:
    ///  - the physical address is invalid or not aligned;
    ///  - any of the frames cannot be created with a specific reason.
    ///
    /// # Panics
    ///
    /// It panics if the range is empty.
    pub fn from_unused<F>(range: Range<Paddr>, mut metadata_fn: F) -> Result<Self, GetFrameError>
    where
        F: FnMut(Paddr) -> M,
    {
        if range.start % PAGE_SIZE != 0 || range.end % PAGE_SIZE != 0 {
            return Err(GetFrameError::NotAligned);
        }
        if range.end > super::max_paddr() {
            return Err(GetFrameError::OutOfBound);
        }
        assert!(range.start < range.end);
        // Construct a segment early to recycle previously forgotten frames if
        // the subsequent operations fails in the middle.
        let mut segment = Self {
            range: range.start..range.start,
            _marker: core::marker::PhantomData,
        };
        for paddr in range.step_by(PAGE_SIZE) {
            let frame = Frame::<M>::from_unused(paddr, metadata_fn(paddr))?;
            let _ = ManuallyDrop::new(frame);
            segment.range.end = paddr + PAGE_SIZE;
        }
        Ok(segment)
    }

    /// Restores the [`Segment`] from the raw physical address range.
    ///
    /// # Safety
    ///
    /// The range must be a forgotten [`Segment`] that matches the type `M`.
    /// It could be manually forgotten by [`core::mem::forget`],
    /// [`ManuallyDrop`], or [`Self::into_raw`].
    pub(crate) unsafe fn from_raw(range: Range<Paddr>) -> Self {
        debug_assert_eq!(range.start % PAGE_SIZE, 0);
        debug_assert_eq!(range.end % PAGE_SIZE, 0);
        Self {
            range,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Segment<M> {
    /// Gets the start physical address of the contiguous frames.
    pub fn start_paddr(&self) -> Paddr {
        self.range.start
    }

    /// Gets the end physical address of the contiguous frames.
    pub fn end_paddr(&self) -> Paddr {
        self.range.end
    }

    /// Gets the length in bytes of the contiguous frames.
    pub fn size(&self) -> usize {
        self.range.end - self.range.start
    }

    /// Splits the frames into two at the given byte offset from the start.
    ///
    /// The resulting frames cannot be empty. So the offset cannot be neither
    /// zero nor the length of the frames.
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

    /// Gets an extra handle to the frames in the byte offset range.
    ///
    /// The sliced byte offset range in indexed by the offset from the start of
    /// the contiguous frames. The resulting frames holds extra reference counts.
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
            // SAFETY: We already have reference counts for the frames since
            // for each frame there would be a forgotten handle when creating
            // the `Segment` object.
            unsafe { inc_frame_ref_count(paddr) };
        }

        Self {
            range: start..end,
            _marker: core::marker::PhantomData,
        }
    }

    /// Forgets the [`Segment`] and gets a raw range of physical addresses.
    pub(crate) fn into_raw(self) -> Range<Paddr> {
        let range = self.range.clone();
        let _ = ManuallyDrop::new(self);
        range
    }
}

impl<M: AnyFrameMeta + ?Sized> From<Frame<M>> for Segment<M> {
    fn from(frame: Frame<M>) -> Self {
        let pa = frame.start_paddr();
        let _ = ManuallyDrop::new(frame);
        Self {
            range: pa..pa + PAGE_SIZE,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Iterator for Segment<M> {
    type Item = Frame<M>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.start < self.range.end {
            // SAFETY: each frame in the range would be a handle forgotten
            // when creating the `Segment` object.
            let frame = unsafe { Frame::<M>::from_raw(self.range.start) };
            self.range.start += PAGE_SIZE;
            // The end cannot be non-page-aligned.
            debug_assert!(self.range.start <= self.range.end);
            Some(frame)
        } else {
            None
        }
    }
}

impl<M: AnyFrameMeta> From<Segment<M>> for Segment<dyn AnyFrameMeta> {
    fn from(seg: Segment<M>) -> Self {
        let seg = ManuallyDrop::new(seg);
        Self {
            range: seg.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta> TryFrom<Segment<dyn AnyFrameMeta>> for Segment<M> {
    type Error = Segment<dyn AnyFrameMeta>;

    fn try_from(seg: Segment<dyn AnyFrameMeta>) -> core::result::Result<Self, Self::Error> {
        // SAFETY: for each page there would be a forgotten handle
        // when creating the `Segment` object.
        let first_frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !(first_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            return Err(seg);
        }
        // Since segments are homogeneous, we can safely assume that the rest
        // of the frames are of the same type. We just debug-check here.
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!((frame.dyn_meta() as &dyn core::any::Any).is::<M>());
            }
        }
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        Ok(unsafe { core::mem::transmute::<Segment<dyn AnyFrameMeta>, Segment<M>>(seg) })
    }
}

impl<M: AnyUFrameMeta> From<Segment<M>> for USegment {
    fn from(seg: Segment<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(seg) }
    }
}

impl TryFrom<Segment<dyn AnyFrameMeta>> for USegment {
    type Error = Segment<dyn AnyFrameMeta>;

    /// Try converting a [`Segment<dyn AnyFrameMeta>`] into [`USegment`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(seg: Segment<dyn AnyFrameMeta>) -> core::result::Result<Self, Self::Error> {
        // SAFETY: for each page there would be a forgotten handle
        // when creating the `Segment` object.
        let first_frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !first_frame.dyn_meta().is_untyped() {
            return Err(seg);
        }
        // Since segments are homogeneous, we can safely assume that the rest
        // of the frames are of the same type. We just debug-check here.
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!(frame.dyn_meta().is_untyped());
            }
        }
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        Ok(unsafe { core::mem::transmute::<Segment<dyn AnyFrameMeta>, USegment>(seg) })
    }
}
