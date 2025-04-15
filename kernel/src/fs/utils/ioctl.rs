// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum IoctlCmd {
    /// Get terminal attributes
    TCGETS = 0x5401,
    TCSETS = 0x5402,
    /// Drain the output buffer and set attributes
    TCSETSW = 0x5403,
    /// Drain the output buffer, and discard pending input, and set attributes
    TCSETSF = 0x5404,
    /// Make the given terminal the controlling terminal of the calling process.
    TIOCSCTTY = 0x540e,
    /// Get the process group ID of the foreground process group on this terminal
    TIOCGPGRP = 0x540f,
    /// Set the foreground process group ID of this terminal.
    TIOCSPGRP = 0x5410,
    /// Get the number of bytes in the input buffer.
    FIONREAD = 0x541B,
    /// Set window size
    TIOCGWINSZ = 0x5413,
    TIOCSWINSZ = 0x5414,
    /// Enable or disable non-blocking I/O mode.
    FIONBIO = 0x5421,
    /// the calling process gives up this controlling terminal
    TIOCNOTTY = 0x5422,
    /// Return the session ID of FD
    TIOCGSID = 0x5429,
    /// Clear the close on exec flag on a file descriptor
    FIONCLEX = 0x5450,
    /// Set the close on exec flag on a file descriptor
    FIOCLEX = 0x5451,
    /// Enable or disable asynchronous I/O mode.
    FIOASYNC = 0x5452,
    /// Get Pty Number
    TIOCGPTN = 0x80045430,
    /// Lock/unlock Pty
    TIOCSPTLCK = 0x40045431,
    /// Safely open the slave
    TIOCGPTPEER = 0x40045441,
    /// font operations
    KDFONTOP = 0x4B72,
    /// Get tdx report using TDCALL
    TDXGETREPORT = 0xc4405401,
    /// Equivalent to FBIOGET_VSCREENINFO
    GETVSCREENINFO = 0x4600,
    /// Equivalent to FBIOPUT_VSCREENINFO
    PUTVSCREENINFO = 0x4601,
    /// Equivalent to FBIOGET_FSCREENINFO
    GETFSCREENINFO = 0x4602,
    /// Equivalent to FBIOGETCMAP
    GETCMAP = 0x4604,
    /// Equivalent to FBIOPUTCMAP
    PUTCMAP = 0x4605,
    /// Equivalent to FBIOPAN_DISPLAY
    PANDISPLAY = 0x4606,
    /// Equivalent to FBIOBLANK
    FBIOBLANK = 0x4611,
}
