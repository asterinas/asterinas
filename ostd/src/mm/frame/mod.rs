// SPDX-License-Identifier: MPL-2.0

//! Untyped physical memory management.
//!
//! A frame is a special page that is _untyped_ memory.
//! It is used to store data irrelevant to the integrity of the kernel.
//! All pages mapped to the virtual address space of the users are backed by
//! frames. Frames, with all the properties of pages, can additionally be safely
//! read and written by the kernel or the user.

pub mod options;
mod segment;

use core::{any::Any, fmt::Debug, mem::ManuallyDrop};

pub use segment::Segment;

use crate::{
    mm::{
        io::{FallibleVmRead, FallibleVmWrite, VmIo, VmReader, VmWriter},
        paddr_to_vaddr,
        page::{
            meta::{FrameMetaBox, MetaSlot, PageMeta, PageUsage},
            DynPage, Page,
        },
        HasPaddr, Infallible, Paddr, PAGE_SIZE,
    },
    Error, Result,
};

/// Accessors for a page of untyped memory.
///
/// Untyped memory allows the kernel to read from it and write to it safely.
/// Also, it can be shared to the user space.
///
/// # Safety
///
/// Implementors of this trait must ensure that the object represents a valid
/// page of untyped memory.
pub unsafe trait UntypedPage {
    /// Returns the physical address of the page frame.
    fn start_paddr(&self) -> Paddr;

    /// Returns the end physical address of the page frame.
    fn end_paddr(&self) -> Paddr {
        self.start_paddr() + self.size()
    }

    /// Returns the size of the frame
    fn size(&self) -> usize {
        PAGE_SIZE
    }

    /// Copies the content of `src` to the frame.
    fn copy_from(&self, src: &Self) {
        if self.start_paddr() == src.start_paddr() {
            return;
        }
        // SAFETY: the source and the destination does not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.size());
        }
    }

    /// Get a [`VmReader`] for the frame that reads from the beginning to the end.
    fn reader(&self) -> VmReader<'_, Infallible> {
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The frame is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the frame.
        unsafe { VmReader::from_kernel_space(self.as_ptr(), self.size()) }
    }

    /// Get a [`VmWriter`] for the frame that writes from the beginning to the end.
    fn writer(&self) -> VmWriter<'_, Infallible> {
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The frame is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the frame.
        unsafe { VmWriter::from_kernel_space(self.as_mut_ptr(), self.size()) }
    }

    #[doc(hidden)]
    fn as_ptr(&self) -> *const u8 {
        paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    #[doc(hidden)]
    fn as_mut_ptr(&self) -> *mut u8 {
        paddr_to_vaddr(self.start_paddr()) as *mut u8
    }
}

/// A handle to a physical memory page of untyped memory.
///
/// An instance of [`Frame`] is a handle to a page frame (a physical memory
/// page). A cloned [`Frame`] refers to the same page frame as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other. Behind the scene, a reference
/// counter is maintained for each page frame so that when all instances of
/// [`Frame`] that refer to the same page frame are dropped, the page frame
/// will be globally freed.
///
/// Any type of metadata `M` can be associated with a frame. The metadata is
/// globally accessible for each frame. By default, the metadata is `()`.
/// Also, [`AnyFrame`] can be used to refer to a frame with the type of
/// metadata unknown at compile time.
///
/// To allocate a new frame, use [`FrameAllocator`].
///
/// # Examples
///
/// ```rust
/// use ostd::mm::{Frame, FrameAllocator};
/// use core::sync::atomic::{AtomicU64, Ordering};
/// struct Metadata {
///     counter: AtomicU64,
/// }
///
/// // Allocate a new frame with metadata
/// let metadata = Metadata { counter: AtomicU64::new(0) };
/// let frame = FrameAllocator::lock().alloc_single(metadata).unwrap();
///
/// // Get the metadata
/// let count = frame.metadata().counter.load(Ordering::Relaxed);
///
/// // Write to the frame
/// frame.writer().write_val(count).unwrap();
/// ```
///
/// [`FrameAllocator`]: crate::mm::FrameAllocator
#[repr(transparent)]
#[derive(Debug)]
pub struct Frame<M: Send + Sync + 'static = ()> {
    page: Page<FrameMetaBox>,
    _marker: PhantomData<M>,
}

// `#[derive(Clone)]` won't work because `M` is not `Clone`. We can clone the
// handle because they point to the same copy of metadata.
impl<M: Send + Sync + 'static> Clone for Frame<M> {
    fn clone(&self) -> Self {
        Self {
            page: self.page.clone(),
            _marker: PhantomData,
        }
    }
}

// SAFETY: A `Frame` can only be allocated from a `FrameAllocator` or casted
// from `AnyFrame` , so it must be a valid page that is not used for other
// typed purposes.
unsafe impl<M: Send + Sync + 'static> UntypedPage for Frame<M> {
    fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }
}

impl<M: Send + Sync + 'static> Frame<M> {
    /// Get the metadata of the frame that was set during the allocation.
    pub fn metadata(&self) -> &M {
        // SAFETY: The metadata stored in `FrameMetaBox` is `M` if the object exists.
        unsafe { self.page.meta().inner().downcast_ref_unchecked::<M>() }
    }

    /// Get the reference count of the frame.
    ///
    /// It returns the number of all references to the page, including all the
    /// existing page handles ([`Frame`]) and all the mappings in the page
    /// table that points to the page.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.page.reference_count()
    }

    /// Cast into a frame with a static metadata type.
    ///
    /// # Safety
    ///
    /// The user must ensure that the metadata type `M` correctly matches the
    /// metadata type of the provided [`AnyFrame`].
    pub(crate) unsafe fn from_unchecked(page: AnyFrame) -> Self {
        core::mem::transmute(page)
    }
}

impl<M: Send + Sync + 'static> TryFrom<AnyFrame> for Frame<M> {
    type Error = AnyFrame;

    /// Try converting a [`AnyFrame`] into the statically-typed [`Frame<M>`].
    ///
    /// If the metadata of the dynamic frame does not match the type `M`, it
    /// will return the dynamic frame itself as is.
    fn try_from(frame: AnyFrame) -> core::result::Result<Self, Self::Error> {
        if frame.metadata().downcast_ref::<M>().is_some() {
            // SAFETY: The metadata type `M` matches the metadata of the frame.
            Ok(unsafe { Self::from_unchecked(frame) })
        } else {
            Err(frame)
        }
    }
}

/// A [`Frame`] with the type of metadata not known at compile time.
///
/// Most of the accessor methods for [`Frame`] are available for [`AnyFrame`]
/// as well. However, the metadata is not accessible directly. To access the
/// metadata, the frame must be converted to a statically-typed [`Frame`] using
/// the [`TryFrom`] trait.
#[repr(transparent)]
#[derive(Clone, Debug)]
pub struct AnyFrame {
    page: Page<FrameMetaBox>,
    _marker: PhantomData<()>,
}

// SAFETY: A `AnyFrame` can only be casted from `Page<FrameMetaBox>` or a
// `DynPage` that is surely untyped page, so it must be a valid page that is
// not used for other typed purposes.
unsafe impl UntypedPage for AnyFrame {
    fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }
}

impl AnyFrame {
    /// Get the reference count of the frame.
    ///
    /// It returns the number of all references to the page, including all the
    /// existing page handles ([`Frame`]) and all the mappings in the page
    /// table that points to the page.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.page.reference_count()
    }

    /// Get the metadata of the frame that was set during the allocation.
    pub fn metadata(&self) -> &dyn Any {
        self.page.meta().inner()
    }
}

impl From<Page<FrameMetaBox>> for AnyFrame {
    fn from(page: Page<FrameMetaBox>) -> Self {
        Self {
            page,
            _marker: PhantomData,
        }
    }
}

impl From<AnyFrame> for Page<FrameMetaBox> {
    fn from(frame: AnyFrame) -> Self {
        frame.page
    }
}

impl<M: Send + Sync + 'static> From<Frame<M>> for Page<FrameMetaBox> {
    fn from(frame: Frame<M>) -> Self {
        frame.page
    }
}

impl<M: Send + Sync + 'static> From<Frame<M>> for AnyFrame {
    fn from(frame: Frame<M>) -> Self {
        // SAFETY: The layouts are the same and transparent. Converting it back
        // we just harmlessly lose the metadata type information.
        unsafe { core::mem::transmute(frame) }
    }
}

impl<'a, M: Send + Sync + 'static> From<&'a Frame<M>> for &'a AnyFrame {
    fn from(frame: &'a Frame<M>) -> Self {
        // SAFETY: The layouts are the same and transparent. Converting it back
        // we just harmlessly lose the metadata type information.
        unsafe { core::mem::transmute(frame) }
    }
}

impl TryFrom<DynPage> for AnyFrame {
    type Error = DynPage;

    /// Try converting a [`DynPage`] into the statically-typed [`Frame`].
    ///
    /// If the dynamic page is not used as an untyped page frame, it will
    /// return the dynamic page itself as is.
    fn try_from(page: DynPage) -> core::result::Result<Self, Self::Error> {
        page.try_into().map(|p: Page<FrameMetaBox>| p.into())
    }
}

impl<T: UntypedPage> HasPaddr for T {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

// We cannot `impl<T: UntypedPage + Send + Sync> VmIo for T`, because that will
// lead to conflicting implementations.

impl<M: Send + Sync + 'static> VmIo for Frame<M> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        vm_read(self, offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        vm_write(self, offset, reader)
    }
}

impl VmIo for AnyFrame {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        vm_read(self, offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        vm_write(self, offset, reader)
    }
}

fn vm_read<T: UntypedPage>(self_: &T, offset: usize, writer: &mut VmWriter) -> Result<()> {
    let read_len = writer.avail().min(self_.size().saturating_sub(offset));
    // Do bound check with potential integer overflow in mind
    let max_offset = offset.checked_add(read_len).ok_or(Error::Overflow)?;
    if max_offset > self_.size() {
        return Err(Error::InvalidArgs);
    }
    let len = self_
        .reader()
        .skip(offset)
        .read_fallible(writer)
        .map_err(|(e, _)| e)?;
    debug_assert!(len == read_len);
    Ok(())
}

fn vm_write<T: UntypedPage>(self_: &T, offset: usize, reader: &mut VmReader) -> Result<()> {
    let write_len = reader.remain().min(self_.size().saturating_sub(offset));
    // Do bound check with potential integer overflow in mind
    let max_offset = offset.checked_add(write_len).ok_or(Error::Overflow)?;
    if max_offset > self_.size() {
        return Err(Error::InvalidArgs);
    }
    let len = self_
        .writer()
        .skip(offset)
        .write_fallible(reader)
        .map_err(|(e, _)| e)?;
    debug_assert!(len == write_len);
    Ok(())
}

impl PageMeta for FrameMetaBox {
    const USAGE: PageUsage = PageUsage::Frame;

    fn on_drop(_page: &mut Page<Self>) {
        // Nothing should be done so far since dropping the page would
        // have all taken care of.
    }
}

// Here are implementations for `xarray`.

use core::{marker::PhantomData, ops::Deref};

/// `FrameRef` is a struct that can work as `&'a AnyFrame`.
///
/// This is solely useful for [`crate::collections::xarray`].
pub struct FrameRef<'a> {
    inner: ManuallyDrop<AnyFrame>,
    _marker: PhantomData<&'a AnyFrame>,
}

impl<'a> Deref for FrameRef<'a> {
    type Target = AnyFrame;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `AnyFrame` is essentially an `*const MetaSlot` that could be used as
// a `*const` pointer. The pointer is also aligned to 4.
unsafe impl xarray::ItemEntry for AnyFrame {
    type Ref<'a> = FrameRef<'a> where Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = self.page.into_raw_ptr();
        ptr as *const ()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            page: Page::<FrameMetaBox>::from_raw_ptr(raw as *const MetaSlot),
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(AnyFrame::from_raw(raw)),
            _marker: PhantomData,
        }
    }
}
