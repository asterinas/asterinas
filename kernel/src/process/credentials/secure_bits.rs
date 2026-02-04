// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU16, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;

use crate::prelude::*;

bitflags! {
    /// Represents the secure bits for a POSIX thread.
    ///
    /// These flags control the behavior of capabilities when changing UIDs.
    ///
    /// Reference: <https://man7.org/linux/man-pages/man7/capabilities.7.html>
    pub struct SecureBits: u16 {
        /// If set, the kernel does not grant capabilities when a set-user-ID-root program
        /// is executed, or when a process with an effective or real UID of 0 calls `execve`.
        const NOROOT = 1 << 0;
        /// Make `NOROOT` bit immutable (irreversible).
        const NOROOT_LOCKED = 1 << 1;

        /// If set, the kernel does not adjust the process's permitted, effective, and
        /// ambient capability sets when the UIDs are switched between zero and nonzero values.
        const NO_SETUID_FIXUP = 1 << 2;
        /// Make `NO_SETUID_FIXUP` bit immutable (irreversible).
        const NO_SETUID_FIXUP_LOCKED = 1 << 3;

        /// If set, the kernel preserves permitted capabilities across UID changes,
        /// specifically when all UIDs transition from root (0) to non-root values.
        const KEEP_CAPS = 1 << 4;
        /// Make `KEEP_CAPS` bit immutable (irreversible).
        const KEEP_CAPS_LOCKED = 1 << 5;

        /// If set, the kernel will not permit raising ambient capabilities via the
        /// prctl PR_CAP_AMBIENT_RAISE operation.
        const NO_CAP_AMBIENT_RAISE = 1 << 6;
        /// Make `NO_CAP_AMBIENT_RAISE` bit immutable (irreversible).
        const NO_CAP_AMBIENT_RAISE_LOCKED = 1 << 7;
    }
}

impl SecureBits {
    /// Mask of all lock bits.
    const LOCK_MASK: u16 = 0b10101010;
    /// Mask of all valid bits.
    const ALL_VALID_BITS: u16 = Self::LOCK_MASK | (Self::LOCK_MASK >> 1);

    /// Creates a new `SecureBits` with default (empty) settings.
    pub(super) const fn new_empty() -> Self {
        SecureBits::empty()
    }

    pub(super) const fn locked_bits(&self) -> SecureBits {
        Self::from_bits_truncate((self.bits & Self::LOCK_MASK) >> 1)
    }

    // Currently, we never grant capabilities when executing a new program, even when switching to
    // root. Therefore, this flag is not used.
    #[expect(dead_code)]
    pub(super) fn no_root(&self) -> bool {
        self.contains(SecureBits::NOROOT)
    }

    pub(super) fn keep_capabilities(&self) -> bool {
        self.contains(SecureBits::KEEP_CAPS)
    }

    pub(super) fn no_setuid_fixup(&self) -> bool {
        self.contains(SecureBits::NO_SETUID_FIXUP)
    }

    // Currently, ambient capabilities and the PR_CAP_AMBIENT_RAISE operation are not supported.
    // Therefore, this flag is not used.
    #[expect(dead_code)]
    pub(super) fn no_cap_ambient_raise(&self) -> bool {
        self.contains(SecureBits::NO_CAP_AMBIENT_RAISE)
    }
}

impl TryFrom<u16> for SecureBits {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self> {
        if value & !SecureBits::ALL_VALID_BITS != 0 {
            return_errno_with_message!(Errno::EINVAL, "the bits are not valid secure bits");
        }

        Ok(SecureBits { bits: value })
    }
}

impl From<SecureBits> for u16 {
    fn from(value: SecureBits) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(SecureBits, try_from = true, {
    #[derive(Debug)]
    struct AtomicSecureBitsInner(AtomicU16);
});

impl Clone for AtomicSecureBitsInner {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}

/// An atomic wrapper around `SecureBits`.
#[derive(Debug, Clone)]
pub(super) struct AtomicSecureBits {
    inner: AtomicSecureBitsInner,
}

impl AtomicSecureBits {
    /// Creates a new `AtomicSecureBits`.
    pub(super) fn new(bits: SecureBits) -> Self {
        Self {
            inner: AtomicSecureBitsInner::new(bits),
        }
    }

    /// Loads the current `SecureBits` atomically.
    pub(super) fn load(&self, ordering: Ordering) -> SecureBits {
        self.inner.load(ordering)
    }

    /// Attempts to store `SecureBits` atomically.
    ///
    /// Returning an error if one of the bits is locked.
    pub(super) fn try_store(&self, bits: SecureBits, ordering: Ordering) -> Result<()> {
        // A thread can only modify its own secure bits, so there are no
        // race conditions and synchronization concerns.

        let current = self.inner.load(Ordering::Relaxed);
        let locked_bits = current.locked_bits();

        if locked_bits & current != locked_bits & bits {
            return_errno_with_message!(Errno::EPERM, "one or more secure bits are locked");
        }

        if SecureBits::LOCK_MASK & current.bits() & !bits.bits() != 0 {
            return_errno_with_message!(Errno::EPERM, "locked secure bits cannot be unlocked");
        }

        self.inner.store(bits, ordering);

        Ok(())
    }
}
