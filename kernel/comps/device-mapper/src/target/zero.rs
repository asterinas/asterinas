// SPDX-License-Identifier: MPL-2.0

//! The `zero` target.
//!
//! Reads return zero-filled blocks and writes are silently discarded. It is
//! useful as a placeholder backing store and for tests.
//!
//! Reference: Linux `Documentation/admin-guide/device-mapper/zero.rst`.

use aster_block::bio::{BioStatus, BioType, SubmittedBio};
use ostd::mm::VmIo;

use super::{DmTarget, zero_vec};

/// A `zero` target.
#[derive(Debug, Default)]
pub struct ZeroTarget;

impl DmTarget for ZeroTarget {
    fn type_name(&self) -> &'static str {
        "zero"
    }

    fn handle_bio(&self, bio: SubmittedBio, _target_start_sector: u64) {
        match bio.type_() {
            BioType::Read => {
                for segment in bio.segments() {
                    let zeros = zero_vec(segment.nbytes());
                    if segment.inner_dma_slice().write_bytes(0, &zeros).is_err() {
                        bio.complete(BioStatus::IoError);
                        return;
                    }
                }
                bio.complete(BioStatus::Complete);
            }
            // Writes are discarded; a flush has nothing to persist.
            BioType::Write | BioType::Flush => bio.complete(BioStatus::Complete),
        }
    }
}
