// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::prelude::*;

/// Control states related to packet mode.
///
/// Reference: <https://man7.org/linux/man-pages/man2/TIOCPKT.2const.html>.
pub(super) struct PacketCtrl {
    mode: AtomicBool,
    status: SpinLock<PacketStatus>,
}

impl PacketCtrl {
    // Creates a new `PacketCtrl`.
    pub(super) fn new() -> Self {
        Self {
            mode: AtomicBool::new(false),
            status: SpinLock::new(PacketStatus::empty()),
        }
    }

    /// Returns whether packet mode is enabled.
    pub(super) fn mode(&self) -> bool {
        self.mode.load(Ordering::Relaxed)
    }

    /// Sets whether packet mode is enabled.
    pub(super) fn set_mode(&self, mode: bool) {
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/pty.c#L158>.
        let mut status = self.status.lock();

        let old_mode = self.mode.load(Ordering::Relaxed);
        self.mode.store(mode, Ordering::Relaxed);

        if mode && !old_mode {
            *status = PacketStatus::empty();
        }
    }

    /// Sets packet status if packet mode is enabled.
    pub(super) fn set_status(&self, set_packet_status: impl FnOnce(&mut PacketStatus)) -> bool {
        // Fast path: Packet mode is disabled.
        if !self.mode.load(Ordering::Relaxed) {
            return false;
        }

        // Packet mode is enabled.
        let mut packet_status = self.status.lock();
        set_packet_status(&mut packet_status);
        true
    }

    /// Checks if packet mode is enabled and there is pending packet status.
    pub(super) fn has_status(&self) -> bool {
        // Fast path: Packet mode is disabled.
        if !self.mode.load(Ordering::Relaxed) {
            return false;
        }

        // Packet mode is enabled.
        !self.status.lock().is_empty()
    }

    /// Takes out packet status if packet mode is enabled.
    pub(super) fn take_status(&self) -> Option<PacketStatus> {
        // Fast path: Packed mode is disabled.
        if !self.mode.load(Ordering::Relaxed) {
            return None;
        }

        // Packet mode is enabled.
        let mut status = self.status.lock();
        let old_status = *status;
        *status = PacketStatus::empty();
        Some(old_status)
    }
}

bitflags! {
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h#L110>.
    pub(super) struct PacketStatus: u8 {
        const DATA = 0;
        const FLUSHREAD = 1 << 0;
        const FLUSHWRITE = 1 << 1;
        const STOP = 1 << 2;
        const START = 1 << 3;
        const NOSTOP = 1 << 4;
        const DOSTOP = 1 << 5;
        const IOCTL = 1 << 6;
    }
}
