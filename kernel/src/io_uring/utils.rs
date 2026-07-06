// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Stores a completion queue entry payload before it is published.
#[derive(Clone, Copy)]
pub(super) struct Completion {
    pub(super) user_data: u64,
    pub(super) res: i32,
    pub(super) flags: u32,
}

impl Completion {
    pub(super) fn new(user_data: u64, res: i32, flags: u32) -> Self {
        Self {
            user_data,
            res,
            flags,
        }
    }

    pub(super) fn with_error(user_data: u64, err: Error) -> Self {
        Self::new(user_data, -(err.error() as i32), 0)
    }
}
