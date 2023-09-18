use crate::prelude::*;

use super::{
    bio::{BioEnqueueError, BioType, SubmittedBio},
    id::Sid,
};

/// Represents the software staging queue for the `BioRequest` objects.
pub trait BioRequestQueue {
    /// Enqueues a `SubmittedBio` to this queue.
    ///
    /// This `SubmittedBio` will be merged into an existing `BioRequest`, or a new
    /// `BioRequest` will be created from the `SubmittedBio` before being placed
    /// into the queue.
    ///
    /// This method will wake up the waiter if a new `BioRequest` is enqueued.
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError>;

    /// Dequeues a `BioRequest` from this queue.
    ///
    /// This method will wait until one request can be retrieved.
    fn dequeue(&self) -> BioRequest;
}

/// The block I/O request.
pub struct BioRequest {
    /// The type of the I/O
    type_: BioType,
    /// The range of target sectors on the device
    sid_range: Range<Sid>,
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
    /// # Panic
    ///
    /// If the `SubmittedBio` can not be merged, this method will panic.
    pub fn merge_bio(&mut self, rq_bio: SubmittedBio) {
        assert!(self.can_merge(&rq_bio));

        if rq_bio.sid_range().start == self.sid_range.end {
            self.sid_range.end = rq_bio.sid_range().end;
            self.bios.push_back(rq_bio);
        } else {
            self.sid_range.start = rq_bio.sid_range().start;
            self.bios.push_front(rq_bio);
        }
    }
}

impl From<SubmittedBio> for BioRequest {
    fn from(bio: SubmittedBio) -> Self {
        Self {
            type_: bio.type_(),
            sid_range: bio.sid_range().clone(),
            bios: {
                let mut bios = VecDeque::with_capacity(1);
                bios.push_front(bio);
                bios
            },
        }
    }
}
