// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SocketTimeout(Option<Duration>);

impl SocketTimeout {
    pub(crate) fn new(duration: Option<Duration>) -> Self {
        Self(duration)
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        self.0
    }
}
