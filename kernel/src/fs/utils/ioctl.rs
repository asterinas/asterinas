// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

#[expect(clippy::upper_case_acronyms)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    // EVIOCGABS = 0x80184580, // 0x80184580 | (axis)
    EVIOCGBIT = 0x80004520,
    EVIOCGID = 0x80084502,
    EVIOCGKEY = 0x80004518,
    EVIOCGLED = 0x80004519,
    EVIOCGNAME = 0x80004506,
    EVIOCGPHYS = 0x80004507,
    EVIOCGUNIQ = 0x80004508,
    EVIOCGPROP = 0x80004509,
    EVIOCGREP = 0x80084503,
    EVIOCGSW = 0x8000451B,
    EVIOCGVERSION = 0x80044501,
    EVIOCSCLOCKID = 0x400445A0,
    // Raw(u32),
}

// impl IoctlCmd {
//     pub fn as_u32(self) -> u32 {
//         match self {
//             IoctlCmd::TCGETS => 0x5401,
//             IoctlCmd::TCSETS => 0x5402,
//             IoctlCmd::TCSETSW => 0x5403,
//             IoctlCmd::TCSETSF => 0x5404,
//             IoctlCmd::TIOCSCTTY => 0x540e,
//             IoctlCmd::TIOCGPGRP => 0x540f,
//             IoctlCmd::TIOCSPGRP => 0x5410,
//             IoctlCmd::FIONREAD => 0x541B,
//             IoctlCmd::TIOCGWINSZ => 0x5413,
//             IoctlCmd::TIOCSWINSZ => 0x5414,
//             IoctlCmd::FIONBIO => 0x5421,
//             IoctlCmd::TIOCNOTTY => 0x5422,
//             IoctlCmd::TIOCGSID => 0x5429,
//             IoctlCmd::FIONCLEX => 0x5450,
//             IoctlCmd::FIOCLEX => 0x5451,
//             IoctlCmd::FIOASYNC => 0x5452,
//             IoctlCmd::TIOCGPTN => 0x80045430,
//             IoctlCmd::TIOCSPTLCK => 0x40045431,
//             IoctlCmd::TIOCGPTPEER => 0x40045441,
//             IoctlCmd::TDXGETREPORT => 0xc4405401,
//             IoctlCmd::GETVSCREENINFO => 0x4600,
//             IoctlCmd::PUTVSCREENINFO => 0x4601,
//             IoctlCmd::GETFSCREENINFO => 0x4602,
//             IoctlCmd::GETCMAP => 0x4604,
//             IoctlCmd::PUTCMAP => 0x4605,
//             IoctlCmd::PANDISPLAY => 0x4606,
//             IoctlCmd::FBIOBLANK => 0x4611,
//             IoctlCmd::EVIOCGBIT => 0x80004520,
//             IoctlCmd::EVIOCGID => 0x80084502,
//             IoctlCmd::EVIOCGKEY => 0x80004518,
//             IoctlCmd::EVIOCGLED => 0x80004519,
//             IoctlCmd::EVIOCGNAME => 0x80004506,
//             IoctlCmd::EVIOCGPHYS => 0x80004507,
//             IoctlCmd::EVIOCGUNIQ => 0x80004508,
//             IoctlCmd::EVIOCGPROP => 0x80004509,
//             IoctlCmd::EVIOCGREP => 0x80084503,
//             IoctlCmd::EVIOCGSW => 0x8000451B,
//             IoctlCmd::EVIOCGVERSION => 0x80044501,
//             IoctlCmd::EVIOCSCLOCKID => 0x400445A0,
//             IoctlCmd::Raw(val) => val,
//         }
//     }

//     pub fn from_u32(value: u32) -> Self {
//         match value {
//             0x5401 => IoctlCmd::TCGETS,
//             0x5402 => IoctlCmd::TCSETS,
//             0x5403 => IoctlCmd::TCSETSW,
//             0x5404 => IoctlCmd::TCSETSF,
//             0x540e => IoctlCmd::TIOCSCTTY,
//             0x540f => IoctlCmd::TIOCGPGRP,
//             0x5410 => IoctlCmd::TIOCSPGRP,
//             0x541B => IoctlCmd::FIONREAD,
//             0x5413 => IoctlCmd::TIOCGWINSZ,
//             0x5414 => IoctlCmd::TIOCSWINSZ,
//             0x5421 => IoctlCmd::FIONBIO,
//             0x5422 => IoctlCmd::TIOCNOTTY,
//             0x5429 => IoctlCmd::TIOCGSID,
//             0x5450 => IoctlCmd::FIONCLEX,
//             0x5451 => IoctlCmd::FIOCLEX,
//             0x5452 => IoctlCmd::FIOASYNC,
//             0x80045430 => IoctlCmd::TIOCGPTN,
//             0x40045431 => IoctlCmd::TIOCSPTLCK,
//             0x40045441 => IoctlCmd::TIOCGPTPEER,
//             0xc4405401 => IoctlCmd::TDXGETREPORT,
//             0x4600 => IoctlCmd::GETVSCREENINFO,
//             0x4601 => IoctlCmd::PUTVSCREENINFO,
//             0x4602 => IoctlCmd::GETFSCREENINFO,
//             0x4604 => IoctlCmd::GETCMAP,
//             0x4605 => IoctlCmd::PUTCMAP,
//             0x4606 => IoctlCmd::PANDISPLAY,
//             0x4611 => IoctlCmd::FBIOBLANK,
//             0x80004520 => IoctlCmd::EVIOCGBIT,
//             0x80084502 => IoctlCmd::EVIOCGID,
//             0x80004518 => IoctlCmd::EVIOCGKEY,
//             0x80004519 => IoctlCmd::EVIOCGLED,
//             0x80004506 => IoctlCmd::EVIOCGNAME,
//             0x80004507 => IoctlCmd::EVIOCGPHYS,
//             0x80004508 => IoctlCmd::EVIOCGUNIQ,
//             0x80004509 => IoctlCmd::EVIOCGPROP,
//             0x80084503 => IoctlCmd::EVIOCGREP,
//             0x8000451B => IoctlCmd::EVIOCGSW,
//             0x80044501 => IoctlCmd::EVIOCGVERSION,
//             0x400445A0 => IoctlCmd::EVIOCSCLOCKID,
//             _ => IoctlCmd::Raw(value),
//         }
//     }
// }
