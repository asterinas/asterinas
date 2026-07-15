// SPDX-License-Identifier: MPL-2.0

use super::RecvFlags;

/// Output of a successful socket receive operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvOutput {
    len: usize,
    flags: RecvFlags,
}

impl RecvOutput {
    /// Creates an output for a successfully received stream.
    pub(in crate::net) const fn new_for_stream(len: usize) -> Self {
        Self {
            len,
            flags: RecvFlags::empty(),
        }
    }

    /// Creates an output for a successfully received packet.
    pub(in crate::net) fn new_for_packet(
        input_flags: RecvFlags,
        copied_len: usize,
        message_len: usize,
    ) -> Self {
        let len = if input_flags.contains(RecvFlags::MSG_TRUNC) {
            message_len
        } else {
            copied_len
        };

        let flags = if copied_len < message_len {
            RecvFlags::MSG_TRUNC
        } else {
            RecvFlags::empty()
        };

        Self { len, flags }
    }

    /// Returns the message size.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns the output flags.
    pub const fn flags(&self) -> RecvFlags {
        self.flags
    }
}
