// SPDX-License-Identifier: MPL-2.0

//! The `error` target.
//!
//! Every read and write fails with an I/O error. It is used to fence off
//! regions that must never be accessed and to exercise error paths.
//!
//! Reference: Linux `Documentation/admin-guide/device-mapper/zero.rst`
//! (the `error` target is documented alongside `zero`).

use aster_block::bio::{BioStatus, SubmittedBio};

use super::DmTarget;

/// An `error` target.
#[derive(Debug, Default)]
pub struct ErrorTarget;

impl DmTarget for ErrorTarget {
    fn type_name(&self) -> &'static str {
        "error"
    }

    fn handle_bio(&self, bio: SubmittedBio, _target_start_sector: u64) {
        bio.complete(BioStatus::IoError);
    }
}
