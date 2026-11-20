// SPDX-License-Identifier: MPL-2.0

//! Device Mapper block major ownership.

use alloc::sync::Arc;

use aster_block::MajorIdOwner;

use crate::DmError;

/// Manages Device Mapper identities in the current boot session.
pub struct DmManager {
    major: Arc<MajorIdOwner>,
}

impl DmManager {
    /// Creates a manager and dynamically allocates a block major.
    pub fn new() -> Result<Self, DmError> {
        let major = Arc::new(
            aster_block::allocate_major_with_name("device-mapper")
                .map_err(|_| DmError::MajorExhausted)?,
        );
        Ok(Self { major })
    }

    /// Returns the dynamic block major used by this manager.
    pub fn major(&self) -> u16 {
        self.major.get().get()
    }
}
