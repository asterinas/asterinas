// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// A control character; the `cc_t` type in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits-common.h#L5>.
type CCtrlChar = u8;

bitflags! {
    /// The input flags; `c_iflags` bits in Linux.
    #[derive(Pod)]
    #[repr(C)]
    pub(super) struct CInputFlags: u32 {
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits-common.h
        const IGNBRK  = 0x001;			/* Ignore break condition */
        const BRKINT  = 0x002;			/* Signal interrupt on break */
        const IGNPAR  = 0x004;			/* Ignore characters with parity errors */
        const PARMRK  = 0x008;			/* Mark parity and framing errors */
        const INPCK   = 0x010;			/* Enable input parity check */
        const ISTRIP  = 0x020;			/* Strip 8th bit off characters */
        const INLCR   = 0x040;			/* Map NL to CR on input */
        const IGNCR   = 0x080;			/* Ignore CR */
        const ICRNL   = 0x100;			/* Map CR to NL on input */
        const IXANY   = 0x800;			/* Any character will restart after stop */
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h
        const IUCLC   = 0x0200;
        const IXON    = 0x0400;
        const IXOFF   = 0x1000;
        const IMAXBEL = 0x2000;
        const IUTF8   = 0x4000;
    }
}

impl Default for CInputFlags {
    fn default() -> Self {
        Self::ICRNL | Self::IXON
    }
}

bitflags! {
    /// The output flags; `c_oflags` bits in Linux.
    #[repr(C)]
    #[derive(Pod)]
    pub(super) struct COutputFlags: u32 {
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits-common.h#L21
        const OPOST  = 1 << 0;			/* Perform output processing */
        const OLCUC  = 1 << 1;
        const ONLCR  = 1 << 2;
        const OCRNL  = 1 << 3;
        const ONOCR  = 1 << 4;
        const ONLRET = 1 << 5;
        const OFILL  = 1 << 6;
        const OFDEL  = 1 << 7;
    }
}

impl Default for COutputFlags {
    fn default() -> Self {
        Self::OPOST | Self::ONLCR
    }
}

/// The control flags; `c_cflags` bits in Linux.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CCtrlFlags(u32);

impl Default for CCtrlFlags {
    fn default() -> Self {
        let cbaud = CCtrlBaud::B38400 as u32;
        let csize = CCtrlSize::CS8 as u32;
        let c_cflags = cbaud | csize | Self::READ_BIT;
        Self(c_cflags)
    }
}

impl CCtrlFlags {
    const BAUD_MASK: u32 = 0x0000100f;
    const SIZE_MASK: u32 = 0x00000030;
    const READ_BIT: u32 = 0x00000080;

    #[expect(dead_code)]
    pub(super) fn baud(&self) -> Result<CCtrlBaud> {
        let baud = self.0 & Self::BAUD_MASK;
        Ok(CCtrlBaud::try_from(baud)?)
    }

    #[expect(dead_code)]
    pub(super) fn size(&self) -> Result<CCtrlSize> {
        let size = self.0 & Self::SIZE_MASK;
        Ok(CCtrlSize::try_from(size)?)
    }

    #[expect(dead_code)]
    pub(super) fn is_read(&self) -> bool {
        self.0 & Self::READ_BIT != 0
    }
}

/// The size part of the control flags ([`CCtrlFlags`]).
#[repr(u32)]
#[derive(Clone, Copy, TryFromInt)]
pub(super) enum CCtrlSize {
    // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h#L97
    CS5 = 0x00000000,
    CS6 = 0x00000010,
    CS7 = 0x00000020,
    CS8 = 0x00000030,
}

/// The baud part of the control flags ([`CCtrlFlags`]).
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub(super) enum CCtrlBaud {
    // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits-common.h#L30
    B0 = 0x00000000, /* hang up */
    B50 = 0x00000001,
    B75 = 0x00000002,
    B110 = 0x00000003,
    B134 = 0x00000004,
    B150 = 0x00000005,
    B200 = 0x00000006,
    B300 = 0x00000007,
    B600 = 0x00000008,
    B1200 = 0x00000009,
    B1800 = 0x0000000a,
    B2400 = 0x0000000b,
    B4800 = 0x0000000c,
    B9600 = 0x0000000d,
    B19200 = 0x0000000e,
    B38400 = 0x0000000f,
}

bitflags! {
    /// The local flags; `c_lflags` bits in Linux.
    #[repr(C)]
    #[derive(Pod)]
    pub(super) struct CLocalFlags: u32 {
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h#L127
        const ISIG    = 0x00001;
        const ICANON  = 0x00002;
        const XCASE   = 0x00004;
        const ECHO    = 0x00008;
        const ECHOE   = 0x00010;
        const ECHOK   = 0x00020;
        const ECHONL  = 0x00040;
        const NOFLSH  = 0x00080;
        const TOSTOP  = 0x00100;
        const ECHOCTL = 0x00200;
        const ECHOPRT = 0x00400;
        const ECHOKE  = 0x00800;
        const FLUSHO  = 0x01000;
        const PENDIN  = 0x04000;
        const IEXTEN  = 0x08000;
        const EXTPROC = 0x10000;
    }
}

impl Default for CLocalFlags {
    fn default() -> Self {
        Self::ICANON
            | Self::ECHO
            | Self::ISIG
            | Self::ECHOE
            | Self::ECHOK
            | Self::ECHOCTL
            | Self::ECHOKE
            | Self::IEXTEN
    }
}

/// An index for a control character ([`CCtrlChar`]).
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(clippy::upper_case_acronyms)]
pub(super) enum CCtrlCharId {
    // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h#L42
    VINTR = 0,
    VQUIT = 1,
    VERASE = 2,
    VKILL = 3,
    VEOF = 4,
    VTIME = 5,
    VMIN = 6,
    VSWTC = 7,
    VSTART = 8,
    VSTOP = 9,
    VSUSP = 10,
    VEOL = 11,
    VREPRINT = 12,
    VDISCARD = 13,
    VWERASE = 14,
    VLNEXT = 15,
    VEOL2 = 16,
}

impl CCtrlCharId {
    // The special char is from gvisor
    pub(super) const fn default_char(&self) -> u8 {
        const fn control_character(c: char) -> u8 {
            debug_assert!(c as u8 >= b'A');
            c as u8 - b'A' + 1u8
        }

        match self {
            Self::VINTR => control_character('C'),
            Self::VQUIT => control_character('\\'),
            Self::VERASE => b'\x7f',
            Self::VKILL => control_character('U'),
            Self::VEOF => control_character('D'),
            Self::VTIME => b'\0',
            Self::VMIN => 1,
            Self::VSWTC => b'\0',
            Self::VSTART => control_character('Q'),
            Self::VSTOP => control_character('S'),
            Self::VSUSP => control_character('Z'),
            Self::VEOL => b'\0',
            Self::VREPRINT => control_character('R'),
            Self::VDISCARD => control_character('O'),
            Self::VWERASE => control_character('W'),
            Self::VLNEXT => control_character('V'),
            Self::VEOL2 => b'\0',
        }
    }
}

/// The termios; `struct termios` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h#L30>.
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub(super) struct CTermios {
    c_iflags: CInputFlags,
    c_oflags: COutputFlags,
    c_cflags: CCtrlFlags,
    c_lflags: CLocalFlags,
    c_line: CCtrlChar,
    c_cc: [CCtrlChar; Self::NUM_CTRL_CHARS],
}

impl Default for CTermios {
    fn default() -> Self {
        let mut termios = Self {
            c_iflags: CInputFlags::default(),
            c_oflags: COutputFlags::default(),
            c_cflags: CCtrlFlags::default(),
            c_lflags: CLocalFlags::default(),
            c_line: 0,
            c_cc: [CCtrlChar::default(); Self::NUM_CTRL_CHARS],
        };
        *termios.special_char_mut(CCtrlCharId::VINTR) = CCtrlCharId::VINTR.default_char();
        *termios.special_char_mut(CCtrlCharId::VQUIT) = CCtrlCharId::VQUIT.default_char();
        *termios.special_char_mut(CCtrlCharId::VERASE) = CCtrlCharId::VERASE.default_char();
        *termios.special_char_mut(CCtrlCharId::VKILL) = CCtrlCharId::VKILL.default_char();
        *termios.special_char_mut(CCtrlCharId::VEOF) = CCtrlCharId::VEOF.default_char();
        *termios.special_char_mut(CCtrlCharId::VTIME) = CCtrlCharId::VTIME.default_char();
        *termios.special_char_mut(CCtrlCharId::VMIN) = CCtrlCharId::VMIN.default_char();
        *termios.special_char_mut(CCtrlCharId::VSWTC) = CCtrlCharId::VSWTC.default_char();
        *termios.special_char_mut(CCtrlCharId::VSTART) = CCtrlCharId::VSTART.default_char();
        *termios.special_char_mut(CCtrlCharId::VSTOP) = CCtrlCharId::VSTOP.default_char();
        *termios.special_char_mut(CCtrlCharId::VSUSP) = CCtrlCharId::VSUSP.default_char();
        *termios.special_char_mut(CCtrlCharId::VEOL) = CCtrlCharId::VEOL.default_char();
        *termios.special_char_mut(CCtrlCharId::VREPRINT) = CCtrlCharId::VREPRINT.default_char();
        *termios.special_char_mut(CCtrlCharId::VDISCARD) = CCtrlCharId::VDISCARD.default_char();
        *termios.special_char_mut(CCtrlCharId::VWERASE) = CCtrlCharId::VWERASE.default_char();
        *termios.special_char_mut(CCtrlCharId::VLNEXT) = CCtrlCharId::VLNEXT.default_char();
        *termios.special_char_mut(CCtrlCharId::VEOL2) = CCtrlCharId::VEOL2.default_char();
        termios
    }
}

impl CTermios {
    /// The number of the control characters.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h#L9>.
    const NUM_CTRL_CHARS: usize = 19;

    pub(super) fn special_char(&self, id: CCtrlCharId) -> CCtrlChar {
        self.c_cc[id as usize]
    }

    pub(super) fn special_char_mut(&mut self, id: CCtrlCharId) -> &mut CCtrlChar {
        &mut self.c_cc[id as usize]
    }

    /// Returns whether the terminal is in the canonical mode.
    ///
    /// The canonical mode means that the input characters will be handled by lines, not by single
    /// characters.
    pub(super) fn is_canonical_mode(&self) -> bool {
        self.c_lflags.contains(CLocalFlags::ICANON)
    }

    /// Returns whether the input flags contain `ICRNL`.
    ///
    /// The `ICRNL` flag means the `\r` characters in the input should be mapped to `\n`.
    pub(super) fn contains_icrnl(&self) -> bool {
        self.c_iflags.contains(CInputFlags::ICRNL)
    }

    pub(super) fn contains_isig(&self) -> bool {
        self.c_lflags.contains(CLocalFlags::ISIG)
    }

    pub(super) fn contain_echo(&self) -> bool {
        self.c_lflags.contains(CLocalFlags::ECHO)
    }

    pub(super) fn contains_echo_ctl(&self) -> bool {
        self.c_lflags.contains(CLocalFlags::ECHOCTL)
    }

    pub(super) fn contains_iexten(&self) -> bool {
        self.c_lflags.contains(CLocalFlags::IEXTEN)
    }
}

/// A window size; `struct winsize` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termios.h#L15>.
#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub(super) struct CWinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// A font operation; `struct console_font_op` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/kd.h#L159>.
#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub(super) struct CFontOp {
    pub(super) op: u32,
    pub(super) flags: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) charcount: u32,
    pub(super) data: usize,
}

impl CFontOp {
    // https://elixir.bootlin.com/linux/v6.15/source/include/uapi/linux/kd.h#L177
    pub(super) const OP_SET: u32 = 0;
    pub(super) const OP_SET_DEFAULT: u32 = 2;
    pub(super) const OP_SET_TALL: u32 = 4;

    // https://elixir.bootlin.com/linux/v6.15/source/drivers/tty/vt/vt.c#L4711
    pub(super) const MAX_WIDTH: u32 = 64;
    pub(super) const MAX_HEIGHT: u32 = 128;
    pub(super) const MAX_CHARCOUNT: u32 = 512;

    // https://elixir.bootlin.com/linux/v6.15/source/drivers/tty/vt/vt.c#L4721
    pub(super) const NONTALL_VPITCH: u32 = 32;
}
