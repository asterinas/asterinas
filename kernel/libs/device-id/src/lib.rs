// SPDX-License-Identifier: MPL-2.0

//! A module represents Linux kernel device IDs.
//!
//! In Linux, each device is identified by a **major** number and a **minor** number.
//! These two numbers together form a unique identifier for the device, which is used
//! in device files (e.g., `/dev/sda`, `/dev/null`) and system calls like `mknod`.
//!
//! For more information about device ID allocation and usage in Linux, see:
//! <https://www.kernel.org/doc/Documentation/admin-guide/devices.txt>

#![no_std]
#![deny(unsafe_code)]

/// A device ID, containing a major device number and a minor device number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceId {
    major: u32,
    minor: u32,
}

impl DeviceId {
    /// Creates a device ID from the major device number and the minor device number.
    pub fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns the major device number.
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Returns the minor device number.
    pub fn minor(&self) -> u32 {
        self.minor
    }
}

impl DeviceId {
    /// Creates a device ID from the encoded `u64` value.
    ///
    /// See [`as_encoded_u64`] for details about how to encode a device ID to a `u64` value.
    ///
    /// [`as_encoded_u64`]: Self::as_encoded_u64
    pub fn from_encoded_u64(raw: u64) -> Self {
        let major = ((raw >> 32) & 0xffff_f000 | (raw >> 8) & 0x0000_0fff) as u32;
        let minor = ((raw >> 12) & 0xffff_ff00 | raw & 0x0000_00ff) as u32;
        Self::new(major, minor)
    }

    /// Encodes the device ID as a `u64` value.
    ///
    /// The lower 32 bits use the same encoding strategy as Linux. See the Linux implementation at:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/kdev_t.h#L39-L44>.
    ///
    /// If the major or minor device number is too large, the additional bits will be recorded
    /// using the higher 32 bits. Note that as of 2025, the Linux kernel still has no support for
    /// 64-bit device IDs:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/types.h#L18>.
    /// So this encoding follows the implementation in glibc:
    /// <https://github.com/bminor/glibc/blob/632d895f3e5d98162f77b9c3c1da4ec19968b671/bits/sysmacros.h#L26-L34>.
    pub fn as_encoded_u64(&self) -> u64 {
        let major = self.major() as u64;
        let minor = self.minor() as u64;
        ((major & 0xffff_f000) << 32)
            | ((major & 0x0000_0fff) << 8)
            | ((minor & 0xffff_ff00) << 12)
            | (minor & 0x0000_00ff)
    }
}
