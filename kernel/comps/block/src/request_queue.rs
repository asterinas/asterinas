// SPDX-License-Identifier: MPL-2.0

use ostd::sync::{Mutex, WaitQueue};

use super::{
    bio::{BioEnqueueError, BioType, SubmittedBio},
    id::Sid,
};
use crate::prelude::*;

/// A simple block I/O request queue backed by one internal FIFO queue.
///
/// It is a FIFO producer-consumer queue, where the producer (e.g., filesystem)
/// submits requests to the queue, and the consumer (e.g., block device driver)
/// continuously consumes and processes these requests from the queue.
///
/// It supports merging the new request with the front request if if the type
/// is same and the sector range is contiguous.
pub struct BioRequestSingleQueue {
    queue: Mutex<VecDeque<BioRequest>>,
    num_requests: AtomicUsize,
    wait_queue: WaitQueue,
    max_nr_segments_per_bio: usize,
}

impl BioRequestSingleQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::with_max_nr_segments_per_bio(usize::MAX)
    }

    /// Creates an empty queue with the upper bound for the number of segments in a bio.
    pub fn with_max_nr_segments_per_bio(max_nr_segments_per_bio: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            num_requests: AtomicUsize::new(0),
            wait_queue: WaitQueue::new(),
            max_nr_segments_per_bio,
        }
    }

    /// Returns the upper limit for the number of segments per bio.
    pub fn max_nr_segments_per_bio(&self) -> usize {
        self.max_nr_segments_per_bio
    }

    /// Returns the number of requests currently in this queue.
    pub fn num_requests(&self) -> usize {
        self.num_requests.load(Ordering::Relaxed)
    }

    /// Enqueues a `SubmittedBio` to this queue.
    ///
    /// When enqueueing the `SubmittedBio`, try to insert it into the last request if the
    /// type is same and the sector range is contiguous.
    /// Otherwise, creates and inserts a new request for the `SubmittedBio`.
    ///
    /// This method will wake up the waiter if a new `BioRequest` is enqueued.
    pub fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        if bio.segments().len() >= self.max_nr_segments_per_bio {
            return Err(BioEnqueueError::TooBig);
        }

        let mut queue = self.queue.lock();
        if let Some(request) = queue.front_mut() {
            if request.can_merge(&bio)
                && request.num_segments() + bio.segments().len() <= self.max_nr_segments_per_bio
            {
                request.merge_bio(bio);
                return Ok(());
            }
        }

        let new_request = BioRequest::from(bio);
        queue.push_front(new_request);
        self.inc_num_requests();
        drop(queue);

        self.wait_queue.wake_all();
        Ok(())
    }

    /// Dequeues a `BioRequest` from this queue.
    ///
    /// This method will wait until one request can be retrieved.
    pub fn dequeue(&self) -> BioRequest {
        let mut num_requests = self.num_requests();

        loop {
            if num_requests > 0 {
                let mut queue = self.queue.lock();
                if let Some(request) = queue.pop_back() {
                    self.dec_num_requests();
                    return request;
                }
            }

            num_requests = self.wait_queue.wait_until(|| {
                let num_requests = self.num_requests();
                if num_requests > 0 {
                    Some(num_requests)
                } else {
                    None
                }
            });
        }
    }

    fn dec_num_requests(&self) {
        self.num_requests.fetch_sub(1, Ordering::Relaxed);
    }

    fn inc_num_requests(&self) {
        self.num_requests.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for BioRequestSingleQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for BioRequestSingleQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BioRequestSingleQueue")
            .field("num_requests", &self.num_requests())
            .field("queue", &self.queue.lock())
            .finish()
    }
}

/// The block I/O request.
///
/// The advantage of this data structure is to merge several `SubmittedBio`s that are
/// contiguous on the target device's sector address, allowing them to be collectively
/// processed in a queue.
#[derive(Debug)]
pub struct BioRequest {
    /// The type of the I/O
    type_: BioType,
    /// The range of target sectors on the device
    sid_range: Range<Sid>,
    /// The number of segments
    num_segments: usize,
    /// The submitted bios
    bios: VecDeque<SubmittedBio>,
}

impl BioRequest {
    /// Returns the type of the I/O.
    pub fn type_(&self) -> BioType {
        self.type_
    }

    /// Returns the range of sector id on device.
    pub fn sid_range(&self) -> &Range<Sid> {
        &self.sid_range
    }

    /// Returns an iterator to the `SubmittedBio`s.
    pub fn bios(&self) -> impl Iterator<Item = &SubmittedBio> {
        self.bios.iter()
    }

    /// Returns the number of segments.
    pub fn num_segments(&self) -> usize {
        self.num_segments
    }

    /// Returns `true` if can merge the `SubmittedBio`, `false` otherwise.
    pub fn can_merge(&self, rq_bio: &SubmittedBio) -> bool {
        if rq_bio.type_() != self.type_ {
            return false;
        }

        rq_bio.sid_range().start == self.sid_range.end
            || rq_bio.sid_range().end == self.sid_range.start
    }

    /// Merges the `SubmittedBio` into this request.
    ///
    /// The merged `SubmittedBio` can only be placed at the front or back.
    ///
    /// # Panics
    ///
    /// If the `SubmittedBio` can not be merged, this method will panic.
    pub fn merge_bio(&mut self, rq_bio: SubmittedBio) {
        assert!(self.can_merge(&rq_bio));

        let rq_bio_nr_segments = rq_bio.segments().len();

        if rq_bio.sid_range().start == self.sid_range.end {
            self.sid_range.end = rq_bio.sid_range().end;
            self.bios.push_back(rq_bio);
        } else {
            self.sid_range.start = rq_bio.sid_range().start;
            self.bios.push_front(rq_bio);
        }

        self.num_segments += rq_bio_nr_segments;
    }
}

impl From<SubmittedBio> for BioRequest {
    fn from(bio: SubmittedBio) -> Self {
        Self {
            type_: bio.type_(),
            sid_range: bio.sid_range().clone(),
            num_segments: bio.segments().len(),
            bios: {
                let mut bios = VecDeque::with_capacity(1);
                bios.push_front(bio);
                bios
            },
        }
    }
}
