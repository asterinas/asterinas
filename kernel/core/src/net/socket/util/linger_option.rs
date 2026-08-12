// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LingerOption {
    is_on: bool,
    timeout: Duration,
}

impl LingerOption {
    pub(crate) fn new(is_on: bool, timeout: Duration) -> Self {
        Self { is_on, timeout }
    }

    pub(crate) fn is_on(&self) -> bool {
        self.is_on
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }
}
