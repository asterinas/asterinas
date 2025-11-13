// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

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
    /// Safely open the slave (0x40045441)
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
            0x40045441 => Self::TIOCGPTPEER,
            0x4B72 => Self::KDFONTOP,
            0x4B3B => Self::KDGETMODE,
            0x4B3A => Self::KDSETMODE,
            0x4B44 => Self::KDGKBMODE,
            0x4B45 => Self::KDSKBMODE,
            0xc4405401 => Self::TDXGETREPORT,
            raw => {
                return Ok(Self::Others(raw));
            }
        };

        Ok(cmd)
    }
}
