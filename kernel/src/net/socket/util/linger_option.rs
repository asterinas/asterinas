// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

#[derive(Debug, Default, Clone, Copy)]
pub struct LingerOption {
    is_on: bool,
    timeout: Duration,
}

impl LingerOption {
    pub fn new(is_on: bool, timeout: Duration) -> Self {
        Self { is_on, timeout }
    }

    pub fn is_on(&self) -> bool {
        self.is_on
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}
