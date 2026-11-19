// SPDX-License-Identifier: MPL-2.0

//! Reporting backend writeback failures to applications.
//!
//! Backend I/O can fail after a buffered write has returned successfully.
//! [`ErrorRecord`] preserves these failures so later `fsync`, `fdatasync`, or
//! `syncfs` calls can report them. Each open file description keeps an
//! [`ErrorObserver`]: checking it against the record reports failures not yet
//! observed by that description and updates the observer's state.
//! Independent opens have independent observers; duplicated descriptors share one.
//!
//! Recovery and any data rewriting are the application's responsibility.
//! Successful writeback does not clear earlier errors, and
//! multiple failures may coalesce into the latest errno.
//!
//! # State layout
//!
//! ```text
//! +----------------------+----------+------------------+
//! | 31..13: generation   | 12: seen | 11..0: errno     |
//! +----------------------+----------+------------------+
//! |       19 bits        |  1 bit   |     12 bits      |
//! +----------------------+----------+------------------+
//! ```
//!
//! - **errno** stores the latest error code; zero denotes the initial error-free state.
//! - **seen** indicates that a check has reported this state, not that every observer has.
//!   Recording clears it; checking tries to set it so newly sampled observers skip old errors.
//! - **generation** advances when recording after a seen error, distinguishing repeated
//!   failures with the same errno. It wraps, so sufficiently stale observers can miss errors.
//!
//! # Example
//!
//! The observers below belong to independent opens of the same file.
//! Each check models the history-reporting step of `fsync`, assuming its current
//! I/O succeeds. An error from this step is returned to userspace by `fsync`.
//!
//! ```rust
//! use crate::{prelude::Errno, vm::page_cache::ErrorRecord};
//!
//! let local_error_record = ErrorRecord::default();
//! // open("file") twice: each open file description creates its own observer.
//! let mut first = local_error_record.sample();
//! let mut second = local_error_record.sample();
//!
//! // A buffered write succeeds, but its backend I/O later fails.
//! local_error_record.record(Errno::EIO);
//!
//! // fsync(first) reports EIO to userspace. This check marks the record SEEN
//! // and updates first's reporting state, leaving second's state unchanged.
//! let fsync_result = local_error_record.check_and_update(&mut first);
//! assert_eq!(fsync_result.unwrap_err().error(), Errno::EIO);
//!
//! // Another thread using the same open description (also through dup())
//! // shares first's observer, so fsync does not report this EIO again.
//! assert!(local_error_record.check_and_update(&mut first).is_ok());
//!
//! // open("file") after SEEN: this new description skips the old failure.
//! let mut later = local_error_record.sample();
//! assert!(local_error_record.check_and_update(&mut later).is_ok());
//!
//! // fsync(second) also reports EIO: every independent open that predates
//! // the failure must learn about it.
//! let fsync_result = local_error_record.check_and_update(&mut second);
//! assert_eq!(fsync_result.unwrap_err().error(), Errno::EIO);
//! ```
//!
//! `SEEN` records that a check reported the failure, not that data was recovered.
//! Applications remain responsible for recovery after receiving the error.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::prelude::*;

/// A shared record of the latest writeback error, initially error-free.
#[derive(Debug, Default)]
pub(crate) struct ErrorRecord {
    value: AtomicU32,
}

/// An independent observer of one [`ErrorRecord`].
///
/// Tracks the last reported error state, updated by [`ErrorRecord::check_and_update`].
/// Use it only with the record that created it.
///
/// Created only by [`ErrorRecord::sample`] for an existing error record.
/// Absence of a record should be represented by an `Option<ErrorObserver>`.
#[derive(Debug)]
pub(crate) struct ErrorObserver(u32);

// Reference: <https://elixir.bootlin.com/linux/v6.12/source/lib/errseq.c>.
const ERROR_MASK: u32 = (1 << 12) - 1;
const SEEN: u32 = 1 << 12;
const COUNTER_INCREMENT: u32 = 1 << 13;

// Relaxed operations suffice: this record conveys only an error code. Callers
// must publish errors before completing writeback and waking its waiters; the
// writeback completion synchronization provides the necessary ordering.
impl ErrorRecord {
    /// Creates an observer that skips errors already reported.
    ///
    /// Sampling does not consume errors. An error that nobody has observed is
    /// still reported when this observer is first checked, even if recorded before
    /// this call.
    pub(crate) fn sample(&self) -> ErrorObserver {
        let value = self.value.load(Ordering::Relaxed);
        ErrorObserver(if value & SEEN == 0 { 0 } else { value })
    }

    /// Records an error, replacing any previous error.
    ///
    /// Recording the same error again after observation makes it reportable
    /// again. There is no operation that clears the recorded error.
    pub(crate) fn record(&self, error: Errno) {
        let error = error as u32;
        debug_assert!(error != 0 && error <= ERROR_MASK);
        let mut old = self.value.load(Ordering::Relaxed);

        loop {
            let mut new = (old & !(ERROR_MASK | SEEN)) | error;
            if old & SEEN != 0 {
                new = new.wrapping_add(COUNTER_INCREMENT);
            }
            if new == old {
                return;
            }

            match self
                .value
                .compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(current) if current == new => return,
                Err(current) => old = current,
            }
        }
    }

    /// Reports the latest unobserved error and updates the observer's state.
    ///
    /// Updates the reporting state even when returning an error.
    /// Returns success if no new error was observed.
    /// Concurrent errors may coalesce or be reported by a later check.
    pub(crate) fn check_and_update(&self, observer: &mut ErrorObserver) -> Result<()> {
        let old = self.value.load(Ordering::Relaxed);
        if old == observer.0 {
            return Ok(());
        }

        let new = old | SEEN;
        if new != old {
            // A writer may replace this snapshot or another observer may mark
            // it seen. Either way we report our snapshot and leave a newer
            // error, if any, for a subsequent check.
            let _ = self
                .value
                .compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed);
        }
        observer.0 = new;

        let error = Errno::try_from((new & ERROR_MASK) as i32)
            .expect("error records contain only recorded Errno values");
        Err(error.into())
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn empty_record_reports_success() {
        let record = ErrorRecord::default();
        let mut sampled = record.sample();
        assert!(record.check_and_update(&mut sampled).is_ok());
    }

    #[ktest]
    fn observers_report_errors_independently() {
        let record = ErrorRecord::default();
        let mut first = record.sample();
        let mut second = record.sample();
        record.record(Errno::EIO);

        assert_eq!(
            record.check_and_update(&mut first).unwrap_err().error(),
            Errno::EIO
        );
        assert!(record.check_and_update(&mut first).is_ok());
        assert_eq!(
            record.check_and_update(&mut second).unwrap_err().error(),
            Errno::EIO
        );
        assert!(record.check_and_update(&mut second).is_ok());
    }

    #[ktest]
    fn repeated_error_is_reported_after_observation() {
        let record = ErrorRecord::default();
        let mut observer = record.sample();
        for _ in 0..2 {
            record.record(Errno::EIO);
            assert_eq!(
                record.check_and_update(&mut observer).unwrap_err().error(),
                Errno::EIO
            );
            assert!(record.check_and_update(&mut observer).is_ok());
        }
    }

    #[ktest]
    fn unseen_errors_coalesce_to_latest_errno() {
        let record = ErrorRecord::default();
        let mut observer = record.sample();
        record.record(Errno::EIO);
        record.record(Errno::EIO);
        record.record(Errno::ENOSPC);
        assert_eq!(
            record.check_and_update(&mut observer).unwrap_err().error(),
            Errno::ENOSPC
        );
        assert!(record.check_and_update(&mut observer).is_ok());
    }

    #[ktest]
    fn new_observer_reports_unseen_old_error() {
        let record = ErrorRecord::default();
        record.record(Errno::EIO);
        let mut observer = record.sample();
        assert_eq!(
            record.check_and_update(&mut observer).unwrap_err().error(),
            Errno::EIO
        );

        // Exercise an unseen error after the counter has already advanced.
        record.record(Errno::ENOSPC);
        let mut later = record.sample();
        assert_eq!(
            record.check_and_update(&mut later).unwrap_err().error(),
            Errno::ENOSPC
        );
    }

    #[ktest]
    fn new_observer_skips_seen_old_error() {
        let record = ErrorRecord::default();
        let mut observer = record.sample();
        record.record(Errno::EIO);
        assert!(record.check_and_update(&mut observer).is_err());

        let mut later = record.sample();
        assert!(record.check_and_update(&mut later).is_ok());
        record.record(Errno::ENOSPC);
        assert_eq!(
            record.check_and_update(&mut later).unwrap_err().error(),
            Errno::ENOSPC
        );
    }

    #[ktest]
    fn counter_wrap_preserves_recorded_errno() {
        // Seed the last counter value to reach the wrap without half a million
        // observations. The encoded state is otherwise a normal seen error.
        let record = ErrorRecord {
            value: AtomicU32::new(!ERROR_MASK | Errno::EIO as u32),
        };
        let mut observer = record.sample();
        record.record(Errno::ENOSPC);
        assert_eq!(record.value.load(Ordering::Relaxed), Errno::ENOSPC as u32);
        assert_eq!(
            record.check_and_update(&mut observer).unwrap_err().error(),
            Errno::ENOSPC
        );
        assert!(record.check_and_update(&mut observer).is_ok());
    }
}
