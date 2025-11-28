// SPDX-License-Identifier: MPL-2.0

use core::convert::Infallible;

use crate::prelude::*;

/// Raw 32-bit ioctl command word (Linux-style encoding).
///
/// Layout (from LSB to MSB):
/// - bits 0..=7   : command number (nr)
/// - bits 8..=15  : type / magic
/// - bits 16..=29 : size (in bytes)
/// - bits 30..=31 : direction
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RawIoctl(u32);

/// Direction of data transfer for an ioctl command.
///
/// The meaning is from the *user-space* perspective,
/// following Linux's `_IOC_*` macros:
///
/// - `None`  -> `_IOC_NONE`
/// - `Write` -> `_IOC_WRITE` (user → kernel)
/// - `Read`  -> `_IOC_READ`  (kernel → user)
/// - `ReadWrite` -> `_IOC_READ | _IOC_WRITE`
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum IoctlDir {
    None,
    Write,
    Read,
    ReadWrite,
    // Unrecognized (non-standard) bit pattern.
    Other(u8),
}

impl IoctlDir {
    fn from_raw(bits: u8) -> Self {
        match bits {
            0 => IoctlDir::None,
            1 => IoctlDir::Write,
            2 => IoctlDir::Read,
            3 => IoctlDir::ReadWrite,
            other => IoctlDir::Other(other),
        }
    }
}

// Bit widths (match `asm-generic/ioctl.h` on Linux).
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_DIRBITS: u32 = 2;

// Shifts.
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;

// Masks for each field (before shifting).
const IOC_NRMASK: u32 = (1 << IOC_NRBITS) - 1;
const IOC_TYPEMASK: u32 = (1 << IOC_TYPEBITS) - 1;
const IOC_SIZEMASK: u32 = (1 << IOC_SIZEBITS) - 1;
const IOC_DIRMASK: u32 = (1 << IOC_DIRBITS) - 1;

impl TryFrom<u32> for RawIoctl {
    type Error = Infallible;

    #[inline]
    fn try_from(value: u32) -> ::core::result::Result<Self, Self::Error> {
        // Any u32 can be interpreted as a raw ioctl command,
        // so this conversion is infallible.
        Ok(RawIoctl(value))
    }
}

impl RawIoctl {
    /// Direction part (`_IOC_DIR(cmd)`).
    #[inline]
    pub fn dir(&self) -> IoctlDir {
        let bits = ((self.0 >> IOC_DIRSHIFT) & IOC_DIRMASK) as u8;
        IoctlDir::from_raw(bits)
    }

    /// Size part in bytes (`_IOC_SIZE(cmd)`).
    ///
    /// Note: this is just the encoded size field; some legacy ioctl
    /// commands may have incorrect size encodings, just like in C.
    #[inline]
    pub fn size(&self) -> u16 {
        ((self.0 >> IOC_SIZESHIFT) & IOC_SIZEMASK) as u16
    }

    /// Type / magic part (`_IOC_TYPE(cmd)`).
    ///
    /// Often this is an ASCII character like `b'r'` or `b'E'`.
    #[inline]
    pub fn ty(&self) -> u8 {
        ((self.0 >> IOC_TYPESHIFT) & IOC_TYPEMASK) as u8
    }

    /// Command number part (`_IOC_NR(cmd)`).
    #[inline]
    pub fn nr(&self) -> u8 {
        ((self.0 >> IOC_NRSHIFT) & IOC_NRMASK) as u8
    }

    pub fn matches(&self, type_: u8, nr: u8, dir: IoctlDir) -> bool {
        self.ty() == type_ && self.nr() == nr && self.dir() == dir
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IoctlRequest {
    raw: RawIoctl,
    user_ptr: usize,
}

impl IoctlRequest {
    pub fn decode(raw: u32, user_ptr: usize) -> Result<Self> {
        let raw = RawIoctl::try_from(raw).expect("RawIoctl conversion is infallible");
        Ok(Self { raw, user_ptr })
    }

    pub const fn raw(&self) -> RawIoctl {
        self.raw
    }

    pub const fn user_ptr(&self) -> usize {
        self.user_ptr
    }

    pub fn buffer_len(&self) -> usize {
        self.raw.size() as usize
    }

    pub fn direction(&self) -> IoctlDir {
        self.raw.dir()
    }

    pub fn type_id(&self) -> u8 {
        self.raw.ty()
    }

    pub fn number(&self) -> u8 {
        self.raw.nr()
    }
}

#[expect(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy)]
pub enum IoctlCmd {
    /// Get terminal attributes (0x5401)
    TCGETS,
    /// Set terminal attributes (0x5402)
    TCSETS,
    /// Drain the output buffer and set attributes (0x5403)
    TCSETSW,
    /// Drain the output buffer, and discard pending input, and set attributes (0x5404)
    TCSETSF,
    /// Make the given terminal the controlling terminal of the calling process (0x540e).
    TIOCSCTTY,
    /// Get the process group ID of the foreground process group on this terminal (0x540f)
    TIOCGPGRP,
    /// Set the foreground process group ID of this terminal (0x5410).
    TIOCSPGRP,
    /// Get the number of bytes in the input buffer (0x541B).
    FIONREAD,
    /// Get window size (0x5413)
    TIOCGWINSZ,
    /// Set window size (0x5414)
    TIOCSWINSZ,
    /// Enable or disable non-blocking I/O mode (0x5421).
    FIONBIO,
    /// The calling process gives up this controlling terminal (0x5422)
    TIOCNOTTY,
    /// Return the session ID of FD (0x5429)
    TIOCGSID,
    /// Clear the close on exec flag on a file descriptor (0x5450)
    FIONCLEX,
    /// Set the close on exec flag on a file descriptor (0x5451)
    FIOCLEX,
    /// Enable or disable asynchronous I/O mode (0x5452).
    FIOASYNC,
    /// Get Pty Number (0x80045430)
    TIOCGPTN,
    /// Lock/unlock Pty (0x40045431)
    TIOCSPTLCK,
    /// Get pty lock state (0x80045439)
    TIOCGPTLCK,
    /// Safely open the slave (0x5441)
    TIOCGPTPEER,
    /// Font operations (0x4B72)
    KDFONTOP,
    /// Get console mode (0x4B3B)
    KDGETMODE,
    /// Set console mode (0x4B3A)
    KDSETMODE,
    /// Get keyboard mode (0x4B44)
    KDGKBMODE,
    /// Set keyboard mode (0x4B45)
    KDSKBMODE,
    /// Get tdx report using TDCALL (0xc4405401)
    TDXGETREPORT,
    /// Get variable screen information (0x4600)
    GETVSCREENINFO,
    /// Set variable screen information (0x4601)
    PUTVSCREENINFO,
    /// Get fixed screen information (0x4602)
    GETFSCREENINFO,
    /// Get framebuffer color map (0x4604)
    GETCMAP,
    /// Set framebuffer color map (0x4605)
    PUTCMAP,
    /// Pan display to show different part of virtual screen (0x4606)
    PANDISPLAY,
    /// Blank or unblank the framebuffer display (0x4611)
    FBIOBLANK,
    /// Other, device-specific ioctls. Raw command is preserved.
    Others(u32),
}

impl TryFrom<u32> for IoctlCmd {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        let cmd = match value {
            0x5401 => Self::TCGETS,
            0x5402 => Self::TCSETS,
            0x5403 => Self::TCSETSW,
            0x5404 => Self::TCSETSF,
            0x540e => Self::TIOCSCTTY,
            0x540f => Self::TIOCGPGRP,
            0x5410 => Self::TIOCSPGRP,
            0x541B => Self::FIONREAD,
            0x5413 => Self::TIOCGWINSZ,
            0x5414 => Self::TIOCSWINSZ,
            0x5421 => Self::FIONBIO,
            0x5422 => Self::TIOCNOTTY,
            0x5429 => Self::TIOCGSID,
            0x5450 => Self::FIONCLEX,
            0x5451 => Self::FIOCLEX,
            0x5452 => Self::FIOASYNC,
            0x80045430 => Self::TIOCGPTN,
            0x40045431 => Self::TIOCSPTLCK,
            0x80045439 => Self::TIOCGPTLCK,
            0x5441 => Self::TIOCGPTPEER,
            0x4B72 => Self::KDFONTOP,
            0x4B3B => Self::KDGETMODE,
            0x4B3A => Self::KDSETMODE,
            0x4B44 => Self::KDGKBMODE,
            0x4B45 => Self::KDSKBMODE,
            0xc4405401 => Self::TDXGETREPORT,
            0x4600 => Self::GETVSCREENINFO,
            0x4601 => Self::PUTVSCREENINFO,
            0x4602 => Self::GETFSCREENINFO,
            0x4604 => Self::GETCMAP,
            0x4605 => Self::PUTCMAP,
            0x4606 => Self::PANDISPLAY,
            0x4611 => Self::FBIOBLANK,
            raw => {
                return Ok(Self::Others(raw));
            }
        };

        Ok(cmd)
    }
}

/// Macro to define ioctl command types.
///
/// For variable-length data (`[u8]`), generates `user_ptr()` and `buffer_len()` methods.
/// For fixed-size data types, generates only `user_ptr()` method.
#[macro_export]
macro_rules! define_ioctl_cmd {
    // Variable-length data variant.
    ($(#[$meta:meta])* $name:ident, $type_val:expr, $nr:expr, $dir:expr, [u8]) => {
        $(#[$meta])*
        pub struct $name {
            request: $crate::fs::utils::IoctlRequest,
        }

        impl core::convert::TryFrom<$crate::fs::utils::IoctlRequest> for $name {
            type Error = $crate::prelude::Error;

            fn try_from(request: $crate::fs::utils::IoctlRequest) -> $crate::prelude::Result<Self> {
                if !request.raw().matches($type_val as u8, $nr, $dir) {
                    return Err($crate::prelude::Error::with_message(
                        $crate::prelude::Errno::EINVAL,
                        "ioctl command mismatch",
                    ));
                }
                Ok(Self { request })
            }
        }

        impl $name {
            pub fn user_ptr(&self) -> usize {
                self.request.user_ptr()
            }

            pub fn buffer_len(&self) -> usize {
                self.request.buffer_len()
            }
        }
    };

    // Fixed-size data variant.
    ($(#[$meta:meta])* $name:ident, $type_val:expr, $nr:expr, $dir:expr, $data_ty:ty) => {
        $(#[$meta])*
        pub struct $name {
            request: $crate::fs::utils::IoctlRequest,
        }

        impl core::convert::TryFrom<$crate::fs::utils::IoctlRequest> for $name {
            type Error = $crate::prelude::Error;

            fn try_from(request: $crate::fs::utils::IoctlRequest) -> $crate::prelude::Result<Self> {
                let raw = request.raw();
                if !raw.matches($type_val as u8, $nr, $dir) {
                    return Err($crate::prelude::Error::with_message(
                        $crate::prelude::Errno::EINVAL,
                        "ioctl command mismatch",
                    ));
                }
                if raw.size() != core::mem::size_of::<$data_ty>() as u16 {
                    return Err($crate::prelude::Error::with_message(
                        $crate::prelude::Errno::EINVAL,
                        "ioctl command size mismatch",
                    ));
                }
                Ok(Self { request })
            }
        }

        impl $name {
            pub fn user_ptr(&self) -> usize {
                self.request.user_ptr()
            }
        }
    };
}
