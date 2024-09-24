// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;

bitflags! {
    /// Represents a set of Linux capabilities.
    pub struct CapSet: u64 {
        const CHOWN = 1 << 0;
        const DAC_OVERRIDE = 1 << 1;
        const DAC_READ_SEARCH = 1 << 2;
        const FOWNER = 1 << 3;
        const FSETID = 1 << 4;
        const KILL = 1 << 5;
        const SETGID = 1 << 6;
        const SETUID = 1 << 7;
        const SETPCAP = 1 << 8;
        const LINUX_IMMUTABLE = 1 << 9;
        const NET_BIND_SERVICE = 1 << 10;
        const NET_BROADCAST = 1 << 11;
        const NET_ADMIN = 1 << 12;
        const NET_RAW = 1 << 13;
        const IPC_LOCK = 1 << 14;
        const IPC_OWNER = 1 << 15;
        const SYS_MODULE = 1 << 16;
        const SYS_RAWIO = 1 << 17;
        const SYS_CHROOT = 1 << 18;
        const SYS_PTRACE = 1 << 19;
        const SYS_PACCT = 1 << 20;
        const SYS_ADMIN = 1 << 21;
        const SYS_BOOT = 1 << 22;
        const SYS_NICE = 1 << 23;
        const SYS_RESOURCE = 1 << 24;
        const SYS_TIME = 1 << 25;
        const SYS_TTY_CONFIG = 1 << 26;
        const MKNOD = 1 << 27;
        const LEASE = 1 << 28;
        const AUDIT_WRITE = 1 << 29;
        const AUDIT_CONTROL = 1 << 30;
        const SETFCAP = 1 << 31;
        const MAC_OVERRIDE = 1 << 32;
        const MAC_ADMIN = 1 << 33;
        const SYSLOG = 1 << 34;
        const WAKE_ALARM = 1 << 35;
        const BLOCK_SUSPEND = 1 << 36;
        const AUDIT_READ = 1 << 37;
        const PERFMON = 1 << 38;
        const BPF = 1 << 39;
        const CHECKPOINT_RESTORE = 1u64 << 40;
        // ... include other capabilities as needed
    }
}

impl CapSet {
    const MASK: u64 = (1 << (CapSet::most_significant_bit() + 1)) - 1;

    /// Converts the capability set to a `u32`. The higher bits are truncated.
    pub fn as_u32(&self) -> u32 {
        self.bits() as u32
    }

    /// Creates a new `CapSet` with the `SYS_ADMIN` capability set, typically for a root user.
    pub const fn new_root() -> Self {
        CapSet::SYS_ADMIN
    }

    /// The most significant bit in a 64-bit `CapSet` that may be set to represent a Linux capability.
    pub const fn most_significant_bit() -> u8 {
        // CHECKPOINT_RESTORE is the Linux capability with the largest numerical value
        40
    }
}

impl TryFrom<u64> for CapSet {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value & !CapSet::MASK != 0 {
            Err("Invalid CapSet.")
        } else {
            Ok(CapSet { bits: value })
        }
    }
}

impl From<CapSet> for u64 {
    fn from(value: CapSet) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(CapSet, try_from = true, {
    #[derive(Debug)]
    pub(super) struct AtomicCapSet(AtomicU64);
});

impl Clone for AtomicCapSet {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}
