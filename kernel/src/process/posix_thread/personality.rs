// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use bitflags::bitflags;

use super::PosixThread;

bitflags! {
    pub struct Personality: u32 {
        const ADDR_NO_RANDOMIZE = 0x0040000;
    }
}

impl PosixThread {
    /// Returns the personality value of this thread.
    pub fn personality(&self) -> u32 {
        self.personality.load(Ordering::Relaxed)
    }

    /// Sets the personality value of this thread.
    pub fn set_personality(&self, personality: u32) {
        self.personality.store(personality, Ordering::Relaxed);
    }
}
