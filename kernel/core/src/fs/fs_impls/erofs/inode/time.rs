// SPDX-License-Identifier: MPL-2.0

//! Validated inode time with its stored meaning and compatibility projections.
//!
//! The frozen core format selects compact time from the superblock and extended
//! time from the inode fields. Modification-time and legacy change-time meanings
//! remain distinct even though all three visible timestamps use the same value.
//! This stage does not validate data layout or construct a runtime inode.
//!
//! References:
//! <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#modification-time-in-compact-inodes>,
//! <https://github.com/torvalds/linux/blob/5dd1818b15d98d4a20806cd00b1b40320b06004f/fs/erofs/inode.c#L211>.

use core::time::Duration;

use super::{
    super::ondisk::super_block::SuperBlock,
    fields::{InodeFields, UncheckedTimeInput},
};

const NANOSECONDS_PER_SECOND: u32 = 1_000_000_000;

/// A selected inode timestamp bounded for the current signed-seconds ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct InodeTime {
    stored_time: Duration,
    kind: StoredTimeKind,
}

impl InodeTime {
    /// Validates the selected stored time and derives its compatibility projections.
    ///
    /// The caller must pair fields and a superblock from the same filesystem.
    /// Only the selected source is checked. The seconds limit protects the current
    /// stat/statx ABI rather than the wider capacity of `Duration`.
    pub(in super::super) fn from_fields(
        fields: &InodeFields,
        super_block: &SuperBlock,
    ) -> Result<Self, InodeTimeError> {
        let (seconds, nanoseconds) = match fields.unchecked_time_input() {
            UncheckedTimeInput::SuperBlock => {
                (super_block.epoch_seconds(), super_block.fixed_nsec())
            }
            UncheckedTimeInput::Extended {
                seconds,
                nanoseconds,
            } => (seconds, nanoseconds),
        };
        if nanoseconds >= NANOSECONDS_PER_SECOND {
            return Err(InodeTimeError::Corrupted(
                CorruptedTime::InvalidNanoseconds(nanoseconds),
            ));
        }
        i64::try_from(seconds).map_err(|_| {
            InodeTimeError::Unsupported(UnsupportedTime::SecondsOutOfRange(seconds))
        })?;
        // Validated nanoseconds cannot be normalized or overflow the seconds.
        let stored_time = Duration::new(seconds, nanoseconds);
        let kind = if super_block.uses_mtime() {
            StoredTimeKind::ModificationTime
        } else {
            StoredTimeKind::LegacyChangeTime
        };
        Ok(Self { stored_time, kind })
    }

    /// Returns the validated timestamp from the selected on-disk source.
    #[expect(dead_code)]
    pub(in super::super) const fn stored_time(&self) -> Duration {
        self.stored_time
    }

    /// Returns the meaning of the single timestamp stored on disk.
    #[expect(dead_code)]
    pub(in super::super) const fn kind(&self) -> StoredTimeKind {
        self.kind
    }

    /// Returns the access-time projection of the selected timestamp.
    pub(in super::super) const fn atime(&self) -> Duration {
        self.stored_time
    }

    /// Returns the modification-time projection of the selected timestamp.
    pub(in super::super) const fn mtime(&self) -> Duration {
        self.stored_time
    }

    /// Returns the change-time projection of the selected timestamp.
    pub(in super::super) const fn ctime(&self) -> Duration {
        self.stored_time
    }
}

/// The on-disk meaning, independent of the three compatibility projections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum StoredTimeKind {
    ModificationTime,
    LegacyChangeTime,
}

/// A malformed selected time or a value outside the current output range.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum InodeTimeError {
    Corrupted(CorruptedTime),
    Unsupported(UnsupportedTime),
}

/// An invalid nanosecond component retained for diagnostics.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum CorruptedTime {
    InvalidNanoseconds(u32),
}

/// A valid disk integer that the current timestamp ABI cannot represent.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedTime {
    SecondsOutOfRange(u64),
}
