// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct SocketTimeout(Option<Duration>);

impl SocketTimeout {
    pub fn new(duration: Option<Duration>) -> Self {
        Self(duration)
    }

    pub fn duration(&self) -> Option<Duration> {
        self.0
    }
}
