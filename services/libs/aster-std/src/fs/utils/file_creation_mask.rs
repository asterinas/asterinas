// SPDX-License-Identifier: MPL-2.0

/// A mask for the file mode of a newly-created file or directory.
///
/// This mask is always a subset of `0o777`.
pub struct FileCreationMask(u16);

impl FileCreationMask {
    // Creates a new instance, the initial value is `0o777`.
    pub fn new(val: u16) -> Self {
        Self(0o777 & val)
    }

    /// Get a new value.
    pub fn get(&self) -> u16 {
        self.0
    }

    /// Set a new value.
    pub fn set(&mut self, new_mask: u16) -> u16 {
        let new_mask = new_mask & 0o777;
        let old_mask = self.0;
        self.0 = new_mask;
        old_mask
    }
}

impl Default for FileCreationMask {
    fn default() -> Self {
        Self(0o022)
    }
}
