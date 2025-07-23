// SPDX-License-Identifier: MPL-2.0

/// The configuration using for bind to a TCP/UDP port.
pub enum BindPortConfig {
    /// Binds to the specified non-reusable port.
    CanReuse(u16),
    /// Binds to the specified reusable port.
    Specified(u16),
    /// Allocates an ephemeral port to bind.
    Ephemeral(bool),
    /// Reuses the port of the listening socket.
    Backlog(u16),
}

impl BindPortConfig {
    /// Creates new configuration using for bind to a TCP/UDP port.
    pub fn new(port: u16, can_reuse: bool) -> Self {
        match (port, can_reuse) {
            (0, can_reuse) => Self::Ephemeral(can_reuse),
            (_, true) => Self::CanReuse(port),
            (_, false) => Self::Specified(port),
        }
    }

    pub(super) fn can_reuse(&self) -> bool {
        matches!(self, Self::CanReuse(_)) || matches!(self, Self::Ephemeral(true))
    }

    pub(super) fn port(&self) -> Option<u16> {
        match self {
            Self::CanReuse(port) | Self::Specified(port) | Self::Backlog(port) => Some(*port),
            Self::Ephemeral(_) => None,
        }
    }
}
