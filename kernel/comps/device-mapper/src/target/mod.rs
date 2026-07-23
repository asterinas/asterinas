// SPDX-License-Identifier: MPL-2.0

//! Device-mapper targets.
//!
//! A target is the per-segment policy that turns a BIO addressed to the mapped
//! device into concrete I/O on the underlying device(s). The set mirrors the
//! Linux device-mapper targets of the same name:
//!
//! - [`linear`]: remaps a contiguous sector range onto a lower device.
//! - [`zero`]: serves zero-filled reads and discards writes.
//! - [`error`]: fails every read and write.

pub mod error;
pub mod linear;
pub mod zero;

use alloc::vec::Vec;

use aster_block::bio::{BioStatus, SubmittedBio};

/// A device-mapper target.
///
/// Each table segment owns one target. The mapped device locates the segment
/// covering an incoming BIO, translates the BIO's starting sector into the
/// target-local coordinate (`target_start_sector`, i.e. the sector offset from
/// the start of the segment), and hands the BIO to [`DmTarget::handle_bio`].
///
/// Implementations own the underlying device(s) and complete the BIO exactly
/// once, either by forwarding the I/O downwards or by synthesizing a result.
pub trait DmTarget: core::fmt::Debug + Send + Sync {
    /// The target type name, matching the keyword used in a dm table line
    /// (for example `linear` or `error`).
    fn type_name(&self) -> &'static str;

    /// The size this target requires its segment to have, in 512-byte sectors.
    ///
    /// Targets that derive their geometry from fixed parameters return `Some`
    /// so the parser can reject a table whose declared segment length disagrees
    /// with that geometry. Targets that adapt to any length return `None`.
    fn size_sectors(&self) -> Option<u64> {
        None
    }

    /// Handles a BIO that falls entirely within this target's segment.
    ///
    /// `target_start_sector` is the sector offset of the BIO from the start of
    /// the segment. The implementation must complete the BIO.
    fn handle_bio(&self, bio: SubmittedBio, target_start_sector: u64);

    /// Flushes any volatile state held by the target.
    ///
    /// Stateless targets that pass writes straight through have nothing of
    /// their own to flush and report success.
    fn flush(&self) -> BioStatus {
        BioStatus::Complete
    }
}

/// Returns a zero-filled buffer of `len` bytes.
pub(crate) fn zero_vec(len: usize) -> Vec<u8> {
    alloc::vec![0u8; len]
}
