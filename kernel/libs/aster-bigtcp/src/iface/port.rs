// SPDX-License-Identifier: MPL-2.0

/// The configuration using for bind to a TCP/UDP port.
pub enum BindPortConfig {
    /// Binds to the specified non-reusable port.
    CanReuse(u16),
    /// Binds to the specified reusable port.
    Specified(u16),
    /// Allocates an ephemeral port to bind.
    Ephemeral,
}

impl BindPortConfig {
    /// Creates new configuration using for bind to a TCP/UDP port.
    ///
    /// # Panics
    ///
    /// This method will panic if `port` is zero (indicating that an ephemeral port should be
    /// allocated) and `can_use` is true. This makes no sense because new ephemeral ports are
    /// always not reused.
    pub fn new(port: u16, can_reuse: bool) -> Self {
        match (port, can_reuse) {
            (0, _) => {
                assert!(!can_reuse);
                Self::Ephemeral
            }
            (_, true) => Self::CanReuse(port),
            (_, false) => Self::Specified(port),
        }
    }

    pub(super) fn can_reuse(&self) -> bool {
        matches!(self, Self::CanReuse(_))
    }

    pub(super) fn port(&self) -> Option<u16> {
        match self {
            Self::CanReuse(port) | Self::Specified(port) => Some(*port),
            Self::Ephemeral => None,
        }
    }
}
