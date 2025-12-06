// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::sync::LocalIrqDisabled;

use crate::prelude::*;

pub struct PacketCtrl {
    mode: AtomicBool,
    status: SpinLock<PacketStatus, LocalIrqDisabled>,
}

impl PacketCtrl {
    pub fn new() -> Self {
        Self {
            mode: AtomicBool::new(false),
            status: SpinLock::new(PacketStatus::empty()),
        }
    }

    pub fn mode(&self) -> bool {
        self.mode.load(Ordering::Relaxed)
    }

    pub fn set_mode(&self, mode: bool) {
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/pty.c#L158>.
        let mut status = self.status.lock();

        let old_mode = self.mode.swap(mode, Ordering::Relaxed);
        if mode & !old_mode {
            *status = PacketStatus::empty();
        }
    }

    pub fn status(&self) -> &SpinLock<PacketStatus, LocalIrqDisabled> {
        &self.status
    }
}

bitflags! {
    pub struct PacketStatus: u8 {
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
