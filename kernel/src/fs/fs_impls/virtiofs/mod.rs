// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs filesystem support backed by FUSE requests.

mod dir;
mod file;
mod fs;
mod inode;
mod open_handle;

use core::time::Duration;

use aster_fuse::FuseError;

use crate::{
    prelude::{Errno, Error},
    time::{Clock, clocks::MonotonicCoarseClock},
};

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs::VirtioFsType).unwrap();
}

impl From<FuseError> for Error {
    fn from(error: FuseError) -> Self {
        match error {
            FuseError::ResourceAlloc(error) => Error::from(error),
            FuseError::MalformedResponse => {
                Error::with_message(Errno::EIO, "malformed virtiofs response")
            }
            FuseError::PageFault => Error::with_message(Errno::EFAULT, "page fault in virtiofs"),
            FuseError::RemoteError(code) => {
                let errno = code
                    .checked_neg()
                    .and_then(|code| Errno::try_from(code).ok())
                    .unwrap_or(Errno::EIO);
                Error::with_message(errno, "filesystem request failed")
            }
            FuseError::BufferTooSmall | FuseError::LengthOverflow => {
                Error::with_message(Errno::EIO, "FUSE protocol encoding error")
            }
        }
    }
}

/// Computes the absolute monotonic deadline when a FUSE cache entry expires.
fn valid_until(secs: u64, nsecs: u32) -> Duration {
    let extra_secs = (nsecs / 1_000_000_000) as u64;
    let nanos = (nsecs % 1_000_000_000) as u64;
    let valid_duration = Duration::from_secs(secs.saturating_add(extra_secs))
        .saturating_add(Duration::from_nanos(nanos));

    MonotonicCoarseClock::get()
        .read_time()
        .saturating_add(valid_duration)
}
