// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use bitvec::array::BitArray;
use int_to_c_enum::TryFromInt;
use ostd::{
    mm::{
        DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, Infallible, Segment, VmIo,
        VmReader, VmWriter,
    },
    sync::{SpinLock, WaitQueue},
    Error,
};
use spin::Once;

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
        segments: Vec<BioSegment>,
        complete_fn: Option<fn(&SubmittedBio)>,
    ) -> Self {
        let nsectors = segments
            .iter()
            .map(|segment| segment.nsectors().to_raw())
            .sum();

        let inner = Arc::new(BioInner {
            type_,
            sid_range: start_sid..start_sid + nsectors,
            segments,
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
    segments: Vec<BioSegment>,
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
        &self.segments
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

/// `BioSegment` is the basic memory unit of a block I/O request.
#[derive(Debug, Clone)]
pub struct BioSegment {
    inner: Arc<BioSegmentInner>,
}

/// The inner part of `BioSegment`.
// TODO: Decouple `BioSegmentInner` with DMA-related buffers.
#[derive(Debug, Clone)]
struct BioSegmentInner {
    /// Internal DMA slice.
    dma_slice: DmaStreamSlice<DmaStream>,
    /// Whether the segment is allocated from the pool.
    from_pool: bool,
}

/// The direction of a bio request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BioDirection {
    /// Read from the backed block device.
    FromDevice,
    /// Write to the backed block device.
    ToDevice,
}

impl<'a> BioSegment {
    /// Allocates a new `BioSegment` with the wanted blocks count and
    /// the bio direction.
    pub fn alloc(nblocks: usize, direction: BioDirection) -> Self {
        Self::alloc_inner(nblocks, 0, nblocks * BLOCK_SIZE, direction)
    }

    /// The inner function that do the real segment allocation.
    ///
    /// Support two extended parameters:
    /// 1. `offset_within_first_block`: the offset (in bytes) within the first block.
    /// 2. `len`: the exact length (in bytes) of the wanted segment. (May
    ///    less than `nblocks * BLOCK_SIZE`)
    ///
    /// # Panics
    ///
    /// If the `offset_within_first_block` or `len` is not sector aligned,
    /// this method will panic.
    pub(super) fn alloc_inner(
        nblocks: usize,
        offset_within_first_block: usize,
        len: usize,
        direction: BioDirection,
    ) -> Self {
        assert!(
            is_sector_aligned(offset_within_first_block)
                && offset_within_first_block < BLOCK_SIZE
                && is_sector_aligned(len)
                && offset_within_first_block + len <= nblocks * BLOCK_SIZE
        );

        // The target segment is whether from the pool or newly-allocated
        let bio_segment_inner = target_pool(direction)
            .and_then(|pool| pool.alloc(nblocks, offset_within_first_block, len))
            .unwrap_or_else(|| {
                let segment = FrameAllocOptions::new(nblocks)
                    .uninit(true)
                    .alloc_contiguous()
                    .unwrap();
                let dma_stream = DmaStream::map(segment, direction.into(), false).unwrap();
                BioSegmentInner {
                    dma_slice: DmaStreamSlice::new(dma_stream, offset_within_first_block, len),
                    from_pool: false,
                }
            });

        Self {
            inner: Arc::new(bio_segment_inner),
        }
    }

    /// Constructs a new `BioSegment` with a given `Segment` and the bio direction.
    pub fn new_from_segment(segment: Segment, direction: BioDirection) -> Self {
        let len = segment.nbytes();
        let dma_stream = DmaStream::map(segment, direction.into(), false).unwrap();
        Self {
            inner: Arc::new(BioSegmentInner {
                dma_slice: DmaStreamSlice::new(dma_stream, 0, len),
                from_pool: false,
            }),
        }
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.inner.dma_slice.nbytes()
    }

    /// Returns the number of sectors.
    pub fn nsectors(&self) -> Sid {
        Sid::from_offset(self.nbytes())
    }

    /// Returns the number of blocks.
    pub fn nblocks(&self) -> usize {
        self.nbytes().align_up(BLOCK_SIZE) / BLOCK_SIZE
    }

    /// Returns the offset (in bytes) within the first block.
    pub fn offset_within_first_block(&self) -> usize {
        self.inner.dma_slice.offset() % BLOCK_SIZE
    }

    /// Returns the inner DMA slice.
    pub fn inner_dma_slice(&self) -> &DmaStreamSlice<DmaStream> {
        &self.inner.dma_slice
    }

    /// Returns the inner VM segment.
    #[cfg(ktest)]
    pub fn inner_segment(&self) -> &Segment {
        self.inner.dma_slice.stream().vm_segment()
    }

    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> Result<VmReader<'a, Infallible>, Error> {
        self.inner.dma_slice.reader()
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> Result<VmWriter<'a, Infallible>, Error> {
        self.inner.dma_slice.writer()
    }
}

impl VmIo for BioSegment {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), Error> {
        self.inner.dma_slice.read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), Error> {
        self.inner.dma_slice.write(offset, reader)
    }
}

// The timing for free the segment to the pool.
impl Drop for BioSegmentInner {
    fn drop(&mut self) {
        if !self.from_pool {
            return;
        }
        if let Some(pool) = target_pool(self.direction()) {
            pool.free(self);
        }
    }
}

impl BioSegmentInner {
    /// Returns the bio direction.
    fn direction(&self) -> BioDirection {
        match self.dma_slice.stream().direction() {
            DmaDirection::FromDevice => BioDirection::FromDevice,
            DmaDirection::ToDevice => BioDirection::ToDevice,
            _ => unreachable!(),
        }
    }
}

/// A pool of managing segments for block I/O requests.
///
/// Inside the pool, it's a large chunk of `DmaStream` which
/// contains the mapped segment. The allocation/free is done by slicing
/// the `DmaStream`.
// TODO: Use a more advanced allocation algorithm to replace the naive one to improve efficiency.
struct BioSegmentPool {
    pool: DmaStream,
    total_blocks: usize,
    direction: BioDirection,
    manager: SpinLock<PoolSlotManager>,
}

/// Manages the free slots in the pool.
struct PoolSlotManager {
    /// A bit array to manage the occupied slots in the pool (Bit
    /// value 1 represents "occupied"; 0 represents "free").
    /// The total size is currently determined by `POOL_DEFAULT_NBLOCKS`.
    occupied: BitArray<[u8; POOL_DEFAULT_NBLOCKS.div_ceil(8)]>,
    /// The first index of all free slots in the pool.
    min_free: usize,
}

impl BioSegmentPool {
    /// Creates a new pool given the bio direction. The total number of
    /// managed blocks is currently set to `POOL_DEFAULT_NBLOCKS`.
    ///
    /// The new pool will be allocated and mapped for later allocation.
    pub fn new(direction: BioDirection) -> Self {
        let total_blocks = POOL_DEFAULT_NBLOCKS;
        let pool = {
            let segment = FrameAllocOptions::new(total_blocks)
                .uninit(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(segment, direction.into(), false).unwrap()
        };
        let manager = SpinLock::new(PoolSlotManager {
            occupied: BitArray::ZERO,
            min_free: 0,
        });

        Self {
            pool,
            total_blocks,
            direction,
            manager,
        }
    }

    /// Allocates a bio segment with the given count `nblocks`
    /// from the pool.
    ///
    /// Support two extended parameters:
    /// 1. `offset_within_first_block`: the offset (in bytes) within the first block.
    /// 2. `len`: the exact length (in bytes) of the wanted segment. (May
    ///    less than `nblocks * BLOCK_SIZE`)
    ///
    /// If there is no enough space in the pool, this method
    /// will return `None`.
    ///
    /// # Panics
    ///
    /// If the `offset_within_first_block` exceeds the block size, or the `len`
    /// exceeds the total length, this method will panic.
    pub fn alloc(
        &self,
        nblocks: usize,
        offset_within_first_block: usize,
        len: usize,
    ) -> Option<BioSegmentInner> {
        assert!(
            offset_within_first_block < BLOCK_SIZE
                && offset_within_first_block + len <= nblocks * BLOCK_SIZE
        );
        let mut manager = self.manager.lock();
        if nblocks > self.total_blocks - manager.min_free {
            return None;
        }

        // Find the free range
        let (start, end) = {
            let mut start = manager.min_free;
            let mut end = start;
            while end < self.total_blocks && end - start < nblocks {
                if manager.occupied[end] {
                    start = end + 1;
                    end = start;
                } else {
                    end += 1;
                }
            }
            if end - start < nblocks {
                return None;
            }
            (start, end)
        };

        manager.occupied[start..end].fill(true);
        manager.min_free = manager.occupied[end..]
            .iter()
            .position(|i| !i)
            .map(|pos| end + pos)
            .unwrap_or(self.total_blocks);

        let dma_slice = DmaStreamSlice::new(
            self.pool.clone(),
            start * BLOCK_SIZE + offset_within_first_block,
            len,
        );
        let bio_segment = BioSegmentInner {
            dma_slice,
            from_pool: true,
        };
        Some(bio_segment)
    }

    /// Returns an allocated bio segment to the pool,
    /// free the space. This method is not public and should only
    /// be called automatically by `BioSegmentInner::drop()`.
    ///
    /// # Panics
    ///
    /// If the target bio segment is not allocated from the pool
    /// or not the same direction, this method will panic.
    fn free(&self, bio_segment: &BioSegmentInner) {
        assert!(bio_segment.from_pool && bio_segment.direction() == self.direction);
        let (start, end) = {
            let dma_slice = &bio_segment.dma_slice;
            let start = dma_slice.offset().align_down(BLOCK_SIZE) / BLOCK_SIZE;
            let end = (dma_slice.offset() + dma_slice.nbytes()).align_up(BLOCK_SIZE) / BLOCK_SIZE;

            if end <= start || end > self.total_blocks {
                return;
            }
            (start, end)
        };

        let mut manager = self.manager.lock();
        debug_assert!(manager.occupied[start..end].iter().all(|i| *i));
        manager.occupied[start..end].fill(false);
        if start < manager.min_free {
            manager.min_free = start;
        }
    }
}

/// A pool of segments for read bio requests only.
static BIO_SEGMENT_RPOOL: Once<Arc<BioSegmentPool>> = Once::new();
/// A pool of segments for write bio requests only.
static BIO_SEGMENT_WPOOL: Once<Arc<BioSegmentPool>> = Once::new();
/// The default number of blocks in each pool. (16MB each for now)
const POOL_DEFAULT_NBLOCKS: usize = 4096;

/// Initializes the bio segment pool.
pub fn bio_segment_pool_init() {
    BIO_SEGMENT_RPOOL.call_once(|| Arc::new(BioSegmentPool::new(BioDirection::FromDevice)));
    BIO_SEGMENT_WPOOL.call_once(|| Arc::new(BioSegmentPool::new(BioDirection::ToDevice)));
}

/// Gets the target pool with the given `direction`.
fn target_pool(direction: BioDirection) -> Option<&'static Arc<BioSegmentPool>> {
    match direction {
        BioDirection::FromDevice => BIO_SEGMENT_RPOOL.get(),
        BioDirection::ToDevice => BIO_SEGMENT_WPOOL.get(),
    }
}

impl From<BioDirection> for DmaDirection {
    fn from(direction: BioDirection) -> Self {
        match direction {
            BioDirection::FromDevice => DmaDirection::FromDevice,
            BioDirection::ToDevice => DmaDirection::ToDevice,
        }
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
