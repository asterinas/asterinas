// SPDX-License-Identifier: MPL-2.0

//! Helper traits and types for asynchronous I/O.
//!
//! The [`IoCompletion`] trait abstracts one asynchronous I/O operation
//! for which a task can call [`IoCompletion::wait`].
//! A list of `IoCompletion` objects can be held by an [`IoBatch`],
//! which provides an [`IoBatch::wait_all`] method
//! so that a task can wait until the whole batch of asynchronous I/O operations completes.
//!
//! # Examples
//!
//! The common pattern is to pass an `IoBatch` as an out-parameter when submitting I/O.
//! The submission path pushes a completion record
//! only when the operation will complete asynchronously.
//! The caller then waits for all records that were added to the batch.
//!
//! ```
//! use std::sync::Arc;
//!
//! use io_util::{
//!     IoError,
//!     batch::{IoBatch, IoCompletion},
//! };
//!
//! struct ReadCompletion {
//!     result: Result<(), IoError>,
//! }
//!
//! impl IoCompletion for ReadCompletion {
//!     fn wait(&self) -> Result<(), IoError> {
//!         self.result
//!     }
//! }
//!
//! struct MockDevice;
//!
//! impl MockDevice {
//!     fn submit_read(&self, io_batch: &mut IoBatch) -> Result<(), IoError> {
//!         io_batch.push(Arc::new(ReadCompletion { result: Ok(()) }));
//!         Ok(())
//!     }
//!
//!     fn submit_fail_read(&self, io_batch: &mut IoBatch) -> Result<(), IoError> {
//!         io_batch.push(Arc::new(ReadCompletion {
//!             result: Err(IoError::Failed),
//!         }));
//!         Ok(())
//!     }
//! }
//!
//! let device = MockDevice;
//! let mut io_batch = IoBatch::new();
//! device.submit_read(&mut io_batch)?;
//! assert_eq!(io_batch.len(), 1);
//! io_batch.wait_all()?;
//!
//! let mut io_batch = IoBatch::new();
//! device.submit_fail_read(&mut io_batch)?;
//! assert_eq!(io_batch.len(), 1);
//! assert_eq!(io_batch.wait_all(), Err(IoError::Failed));
//!
//! # Ok::<(), IoError>(())
//! ```

extern crate alloc;

use alloc::sync::Arc;
use core::{any::Any, ops};

use smallvec::SmallVec;

use crate::IoError;

/// A handle to one asynchronous I/O operation.
///
/// Implementations block in [`wait`](Self::wait) until the underlying I/O terminates,
/// then return its outcome.
/// They are typically wrapped in an `Arc`
/// because the same record is held by both the submitter (through an [`IoBatch`])
/// and the driver that completes the I/O.
pub trait IoCompletion: Send + Sync + Any {
    /// Waits for the I/O operation to complete.
    fn wait(&self) -> Result<(), IoError>;
}

impl dyn IoCompletion {
    /// Returns a reference to the concrete completion type, if it matches `T`.
    pub fn downcast_ref<T: IoCompletion + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

/// A batch of [`IoCompletion`] records.
///
/// Used as an out-parameter on async-submission APIs:
/// callers create an `IoBatch`,
/// pass `&mut` to one or more submission calls,
/// then call [`wait_all`](Self::wait_all)
/// to block until every completion record in the batch has completed.
///
/// `IoBatch` does not track readiness.
/// A completion record is counted by [`len`](Self::len)
/// as long as it is stored in the batch,
/// even if calling [`IoCompletion::wait`] on it would return immediately.
#[must_use]
pub struct IoBatch {
    completions: SmallVec<[Arc<dyn IoCompletion>; INLINE_CAPACITY]>,
}

/// The inline capacity for the common single-completion case.
///
/// Many call sites submit only one completion through an `IoBatch`.
/// Keeping one slot inline avoids heap allocation in that common case.
const INLINE_CAPACITY: usize = 1;

impl IoBatch {
    /// Creates an empty batch.
    pub fn new() -> Self {
        Self {
            completions: SmallVec::new(),
        }
    }

    /// Creates an empty batch with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            completions: SmallVec::with_capacity(capacity),
        }
    }

    /// Adds one completion record to the batch.
    pub fn push(&mut self, completion: Arc<dyn IoCompletion>) {
        self.completions.push(completion);
    }

    /// Returns the number of completion records.
    pub fn len(&self) -> usize {
        self.completions.len()
    }

    /// Returns `true` if the batch has no completion records.
    pub fn is_empty(&self) -> bool {
        self.completions.is_empty()
    }

    /// Waits for every completion record in the batch.
    ///
    /// All completions are waited on even if an earlier one fails.
    /// The first observed error is returned.
    pub fn wait_all(&self) -> Result<(), IoError> {
        let mut first_error = None;

        for completion in &self.completions {
            if let Err(error) = completion.wait() {
                first_error.get_or_insert(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Default for IoBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl ops::Index<usize> for IoBatch {
    type Output = Arc<dyn IoCompletion>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.completions[index]
    }
}
