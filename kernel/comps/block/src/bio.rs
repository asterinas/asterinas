// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use int_to_c_enum::TryFromInt;
use ostd::{
    mm::{FrameAllocOptions, Infallible, UFrame, USegment, UntypedMem, VmIo, VmReader, VmWriter},
    sync::WaitQueue,
    Error,
};

use super::{id::Sid, BlockDevice};
use crate::{prelude::*, BLOCK_SIZE, SECTOR_SIZE};

/// The unit for block I/O.
///
/// Each `Bio` packs the following information:
/// (1) The type of the I/O,
/// (2) The target sectors on the device for doing I/O,
/// (3) The memory locations (`BioSegment`) from/to which data are read/written,
/// (4) The optional callback function that will be invoked when the I/O is completed.
#[derive(Debug)]
pub struct Bio(Arc<BioInner>);

impl Bio {
    /// Constructs a new `Bio`.
    ///
    /// The `type_` describes the type of the I/O.
    /// The `start_sid` is the starting sector id on the device.
    /// The `segments` describes the memory segments.
    /// The `complete_fn` is the optional callback function.
    pub fn new(
        type_: BioType,
        start_sid: Sid,
        bvec: BioVec,
        complete_fn: Option<fn(&SubmittedBio)>,
    ) -> Self {
        let nsectors = bvec.iter().map(|segment| segment.nsectors().to_raw()).sum();

        let inner = Arc::new(BioInner {
            type_,
            sid_range: start_sid..start_sid + nsectors,
            bvec,
            complete_fn,
            status: AtomicU32::new(BioStatus::Init as u32),
            wait_queue: WaitQueue::new(),
        });
        Self(inner)
    }

    /// Returns the type.
    pub fn type_(&self) -> BioType {
        self.0.type_()
    }

    /// Returns the range of target sectors on the device.
    pub fn sid_range(&self) -> &Range<Sid> {
        self.0.sid_range()
    }

    /// Returns the slice to the memory segments.
    pub fn segments(&self) -> &[BioSegment] {
        self.0.segments()
    }

    /// Returns the status.
    pub fn status(&self) -> BioStatus {
        self.0.status()
    }

    /// Submits self to the `block_device` asynchronously.
    ///
    /// Returns a `BioWaiter` to the caller to wait for its completion.
    ///
    /// # Panics
    ///
    /// The caller must not submit a `Bio` more than once. Otherwise, a panic shall be triggered.
    pub fn submit(&self, block_device: &dyn BlockDevice) -> Result<BioWaiter, BioEnqueueError> {
        // Change the status from "Init" to "Submit".
        let result = self.0.status.compare_exchange(
            BioStatus::Init as u32,
            BioStatus::Submit as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        if let Err(e) = block_device.enqueue(SubmittedBio(self.0.clone())) {
            // Fail to submit, revert the status.
            let result = self.0.status.compare_exchange(
                BioStatus::Submit as u32,
                BioStatus::Init as u32,
                Ordering::Release,
                Ordering::Relaxed,
            );
            assert!(result.is_ok());
            return Err(e);
        }

        Ok(BioWaiter {
            bios: vec![self.0.clone()],
        })
    }

    /// Submits self to the `block_device` and waits for the result synchronously.
    ///
    /// Returns the result status of the `Bio`.
    ///
    /// # Panics
    ///
    /// The caller must not submit a `Bio` more than once. Otherwise, a panic shall be triggered.
    pub fn submit_and_wait(
        &self,
        block_device: &dyn BlockDevice,
    ) -> Result<BioStatus, BioEnqueueError> {
        let waiter = self.submit(block_device)?;
        match waiter.wait() {
            Some(status) => {
                assert!(status == BioStatus::Complete);
                Ok(status)
            }
            None => {
                let status = self.status();
                assert!(status != BioStatus::Complete);
                Ok(status)
            }
        }
    }
}

/// The error type returned when enqueueing the `Bio`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BioEnqueueError {
    /// The request queue is full
    IsFull,
    /// Refuse to enqueue the bio
    Refused,
    /// Too big bio
    TooBig,
}

impl From<BioEnqueueError> for ostd::Error {
    fn from(_error: BioEnqueueError) -> Self {
        ostd::Error::NotEnoughResources
    }
}

/// A waiter for `Bio` submissions.
///
/// This structure holds a list of `Bio` requests and provides functionality to
/// wait for their completion and retrieve their statuses.
#[must_use]
#[derive(Debug)]
pub struct BioWaiter {
    bios: Vec<Arc<BioInner>>,
}

impl BioWaiter {
    /// Constructs a new `BioWaiter` instance with no `Bio` requests.
    pub fn new() -> Self {
        Self { bios: Vec::new() }
    }

    /// Returns the number of `Bio` requests associated with `self`.
    pub fn nreqs(&self) -> usize {
        self.bios.len()
    }

    /// Gets the `index`-th `Bio` request associated with `self`.
    ///
    /// # Panics
    ///
    /// If the `index` is out of bounds, this method will panic.
    pub fn req(&self, index: usize) -> Bio {
        Bio(self.bios[index].clone())
    }

    /// Returns the status of the `index`-th `Bio` request associated with `self`.
    ///
    /// # Panics
    ///
    /// If the `index` is out of bounds, this method will panic.
    pub fn status(&self, index: usize) -> BioStatus {
        self.bios[index].status()
    }

    /// Merges the `Bio` requests from another `BioWaiter` into this one.
    ///
    /// The another `BioWaiter`'s `Bio` requests are appended to the end of
    /// the `Bio` list of `self`, effectively concatenating the two lists.
    pub fn concat(&mut self, mut other: Self) {
        self.bios.append(&mut other.bios);
    }

    /// Waits for the completion of all `Bio` requests.
    ///
    /// This method iterates through each `Bio` in the list, waiting for their
    /// completion.
    ///
    /// The return value is an option indicating whether all the requests in the list
    /// have successfully completed.
    /// On success this value is guaranteed to be equal to `Some(BioStatus::Complete)`.
    pub fn wait(&self) -> Option<BioStatus> {
        let mut ret = Some(BioStatus::Complete);

        for bio in self.bios.iter() {
            let status = bio.wait_queue.wait_until(|| {
                let status = bio.status();
                if status != BioStatus::Submit {
                    Some(status)
                } else {
                    None
                }
            });
            if status != BioStatus::Complete && ret.is_some() {
                ret = None;
            }
        }

        ret
    }

    /// Clears all `Bio` requests in this waiter.
    pub fn clear(&mut self) {
        self.bios.clear();
    }
}

impl Default for BioWaiter {
    fn default() -> Self {
        Self::new()
    }
}

/// A submitted `Bio` object.
///
/// The request queue of block device only accepts a `SubmittedBio` into the queue.
#[derive(Debug)]
pub struct SubmittedBio(Arc<BioInner>);

impl SubmittedBio {
    /// Returns the type.
    pub fn type_(&self) -> BioType {
        self.0.type_()
    }

    /// Returns the range of target sectors on the device.
    pub fn sid_range(&self) -> &Range<Sid> {
        self.0.sid_range()
    }

    /// Returns the slice to the memory segments.
    pub fn segments(&self) -> &[BioSegment] {
        self.0.segments()
    }

    /// Returns the status.
    pub fn status(&self) -> BioStatus {
        self.0.status()
    }

    /// Completes the `Bio` with the `status` and invokes the callback function.
    ///
    /// When the driver finishes the request for this `Bio`, it will call this method.
    pub fn complete(&self, status: BioStatus) {
        assert!(status != BioStatus::Init && status != BioStatus::Submit);

        // Set the status.
        let result = self.0.status.compare_exchange(
            BioStatus::Submit as u32,
            status as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        self.0.wait_queue.wake_all();
        if let Some(complete_fn) = self.0.complete_fn {
            complete_fn(self);
        }
    }
}

/// The common inner part of `Bio`.
struct BioInner {
    /// The type of the I/O
    type_: BioType,
    /// The range of the sector id on device
    sid_range: Range<Sid>,
    /// The memory segments in this `Bio`
    // segments: Vec<BioSegment>,
    bvec: BioVec,
    /// The I/O completion method
    complete_fn: Option<fn(&SubmittedBio)>,
    /// The I/O status
    status: AtomicU32,
    /// The wait queue for I/O completion
    wait_queue: WaitQueue,
}

impl BioInner {
    pub fn type_(&self) -> BioType {
        self.type_
    }

    pub fn sid_range(&self) -> &Range<Sid> {
        &self.sid_range
    }

    pub fn segments(&self) -> &[BioSegment] {
        &self.bvec.segments()
    }

    pub fn status(&self) -> BioStatus {
        BioStatus::try_from(self.status.load(Ordering::Relaxed)).unwrap()
    }
}

impl Debug for BioInner {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BioInner")
            .field("type", &self.type_())
            .field("sid_range", &self.sid_range())
            .field("status", &self.status())
            .field("segments", &self.segments())
            .field("complete_fn", &self.complete_fn)
            .finish()
    }
}

/// The type of `Bio`.
#[derive(Clone, Copy, Debug, PartialEq, TryFromInt)]
#[repr(u8)]
pub enum BioType {
    /// Read sectors from the device.
    Read = 0,
    /// Write sectors into the device.
    Write = 1,
    /// Flush the volatile write cache.
    Flush = 2,
    /// Discard sectors.
    Discard = 3,
}

/// The status of `Bio`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, TryFromInt)]
#[repr(u32)]
pub enum BioStatus {
    /// The initial status for a newly created `Bio`.
    Init = 0,
    /// After a `Bio` is submitted, its status will be changed to "Submit".
    Submit = 1,
    /// The I/O operation has been successfully completed.
    Complete = 2,
    /// The I/O operation is not supported.
    NotSupported = 3,
    /// Insufficient space is available to perform the I/O operation.
    NoSpace = 4,
    /// An error occurred while doing I/O.
    IoError = 5,
}

pub struct BioVec {
    segments: Vec<BioSegment>,
}

/// `BioSegment` is the basic memory unit of a block I/O request.
#[derive(Debug, Clone)]
pub struct BioSegment {
    segment: USegment,
    offset: usize,
    len: usize,
}

impl BioVec {
    pub fn new(segments: Vec<BioSegment>) -> Self {
        Self { segments }
    }

    pub fn new_single(segment: BioSegment) -> Self {
        Self {
            segments: vec![segment],
        }
    }

    pub fn empty() -> Self {
        Self { segments: vec![] }
    }

    pub fn segments(&self) -> &[BioSegment] {
        &self.segments
    }

    pub fn iter(&self) -> impl Iterator<Item = &BioSegment> + '_ {
        self.segments.iter()
    }
}

impl BioSegment {
    pub fn new(segment: USegment, offset: usize, len: usize) -> Self {
        Self::new_checked(segment, offset, len).unwrap()
    }

    pub fn new_checked(segment: USegment, offset: usize, len: usize) -> Option<Self> {
        if offset + len <= segment.size() && is_sector_aligned(offset) && is_sector_aligned(len) {
            Some(Self {
                segment,
                offset,
                len,
            })
        } else {
            None
        }
    }

    pub fn alloc(nblocks: usize) -> Result<Self, Error> {
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(nblocks)?
            .into();
        let len = nblocks * BLOCK_SIZE;
        Ok(Self::new(segment, 0, len))
    }

    pub fn alloc_with_offset(offset: usize, len: usize) -> Result<Self, Error> {
        if offset >= BLOCK_SIZE {
            // An offset that exceeds `BLOCK_SIZE` does not make sense.
            return Err(Error::InvalidArgs);
        }
        let nblocks = (offset + len).align_up(BLOCK_SIZE) / BLOCK_SIZE;
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(nblocks)?
            .into();
        Ok(Self::new(segment, offset, len))
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.len
    }

    /// Returns the number of sectors.
    pub fn nsectors(&self) -> Sid {
        Sid::from_offset(self.nbytes())
    }

    /// Returns the number of blocks.
    pub fn nblocks(&self) -> usize {
        self.nbytes().align_up(BLOCK_SIZE) / BLOCK_SIZE
    }

    /// Returns the offset (in bytes).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns a reader to read data from it.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.segment.reader().skip(self.offset).clone()
    }

    /// Returns a writer to write data into it.
    pub fn writer(&self) -> VmWriter<'_, Infallible> {
        todo!()
        // self.segment.writer().skip(self.offset).clone()
    }
}

impl From<USegment> for BioSegment {
    fn from(segment: USegment) -> Self {
        let len = segment.size();
        Self::new(segment, 0, len)
    }
}

impl From<UFrame> for BioSegment {
    fn from(frame: UFrame) -> Self {
        let len = frame.size();
        Self::new(frame.into(), 0, len)
    }
}

impl VmIo for BioSegment {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), Error> {
        self.segment.read(offset + self.offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), Error> {
        self.segment.write(offset + self.offset, reader)
    }
}

/// Checks if the given offset is aligned to sector.
pub fn is_sector_aligned(offset: usize) -> bool {
    offset % SECTOR_SIZE == 0
}

/// An aligned unsigned integer number.
///
/// An instance of `AlignedUsize<const N: u16>` is guaranteed to have a value that is a multiple
/// of `N`, a predetermined const value. It is preferable to express an unsigned integer value
/// in type `AlignedUsize<_>` instead of `usize` if the value must satisfy an alignment requirement.
/// This helps readability and prevents bugs.
///
/// # Examples
///
/// ```rust
/// const SECTOR_SIZE: u16 = 512;
///
/// let sector_num = 1234; // The 1234-th sector
/// let sector_offset: AlignedUsize<SECTOR_SIZE> = {
///     let sector_offset = sector_num * (SECTOR_SIZE as usize);
///     AlignedUsize::<SECTOR_SIZE>::new(sector_offset).unwrap()
/// };
/// assert!(sector_offset.value() % sector_offset.align() == 0);
/// ```
///
/// # Limitation
///
/// Currently, the alignment const value must be expressed in `u16`;
/// it is not possible to use a larger or smaller type.
/// This limitation is inherited from that of Rust's const generics:
/// your code can be generic over the _value_ of a const, but not the _type_ of the const.
/// We choose `u16` because it is reasonably large to represent any alignment value
/// used in practice.
#[derive(Debug, Clone)]
pub struct AlignedUsize<const N: u16>(usize);

impl<const N: u16> AlignedUsize<N> {
    /// Constructs a new instance of aligned integer if the given value is aligned.
    pub fn new(val: usize) -> Option<Self> {
        if val % (N as usize) == 0 {
            Some(Self(val))
        } else {
            None
        }
    }

    /// Returns the value.
    pub fn value(&self) -> usize {
        self.0
    }

    /// Returns the corresponding ID.
    ///
    /// The so-called "ID" of an aligned integer is defined to be `self.value() / self.align()`.
    /// This value is named ID because one common use case is using `Aligned` to express
    /// the byte offset of a sector, block, or page. In this case, the `id` method returns
    /// the ID of the corresponding sector, block, or page.
    pub fn id(&self) -> usize {
        self.value() / self.align()
    }

    /// Returns the alignment.
    pub fn align(&self) -> usize {
        N as usize
    }
}
