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

use aster_util::ranged_integer::{RangedU16, RangedU32};

/// A device ID, embedding the major ID and minor ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceId(u32);

impl DeviceId {
    /// Creates a device ID from the major device number and the minor device number.
    pub fn new(major: MajorId, minor: MinorId) -> Self {
        Self(((major.get() as u32) << 20) | minor.get())
    }

    /// Returns the encoded `u32` value.
    pub fn to_raw(&self) -> u32 {
        self.0
    }

    /// Returns the major device number.
    pub fn major(&self) -> MajorId {
        MajorId::new((self.0 >> 20) as u16)
    }

    /// Returns the minor device number.
    pub fn minor(&self) -> MinorId {
        MinorId::new(self.0 & 0xf_ffff)
    }
}

impl DeviceId {
    /// Creates a device ID from the encoded `u64` value.
    ///
    /// Returns `None` if the major or minor device number is not falling in the valid range.
    pub fn from_encoded_u64(raw: u64) -> Option<Self> {
        let (major, minor) = decode_device_numbers(raw);
        let major = MajorId::try_from(major as u16).ok()?;
        let minor = MinorId::try_from(minor).ok()?;
        Some(Self::new(major, minor))
    }

    /// Encodes the device ID as a `u64` value.
    pub fn as_encoded_u64(&self) -> u64 {
        encode_device_numbers(self.major().get() as u32, self.minor().get())
    }
}

/// Decodes the major and minor numbers from the encoded `u64` value.
///
/// See [`DeviceId::as_encoded_u64`] for details about how to encode a device ID to a `u64` value.
pub fn decode_device_numbers(raw: u64) -> (u32, u32) {
    let major = ((raw >> 32) & 0xffff_f000 | (raw >> 8) & 0x0000_0fff) as u32;
    let minor = ((raw >> 12) & 0xffff_ff00 | raw & 0x0000_00ff) as u32;
    (major, minor)
}

/// Encodes the major and minor numbers as a `u64` value.
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
pub fn encode_device_numbers(major: u32, minor: u32) -> u64 {
    let major = major as u64;
    let minor = minor as u64;
    ((major & 0xffff_f000) << 32)
        | ((major & 0x0000_0fff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0x0000_00ff)
}

const MAX_MAJOR_ID: u16 = 0x0fff;

const MAX_MINOR_ID: u32 = 0x000f_ffff;

/// The major component of a device ID.
///
/// A major ID is a non-zero, 12-bit integer, thus falling in the range of `1..(1u16 << 12)`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/kdev_t.h#L10>.
pub type MajorId = RangedU16<1, MAX_MAJOR_ID>;

/// The minor component of a device ID.
///
/// A minor ID is a 20-bit integer, thus falling in the range of `0..(1u32 << 20)`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/kdev_t.h#L11>.
pub type MinorId = RangedU32<0, MAX_MINOR_ID>;
