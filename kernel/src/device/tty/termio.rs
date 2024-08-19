// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(non_camel_case_types)]

use crate::prelude::*;

// This definition is from occlum
const KERNEL_NCCS: usize = 19;

type TcflagT = u32;
type CcT = u8;
type SpeedT = u32;

bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    pub struct C_IFLAGS: u32 {
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

impl Default for C_IFLAGS {
    fn default() -> Self {
        C_IFLAGS::ICRNL | C_IFLAGS::IXON
    }
}

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct C_OFLAGS: u32 {
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

impl Default for C_OFLAGS {
    fn default() -> Self {
        C_OFLAGS::OPOST | C_OFLAGS::ONLCR
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct C_CFLAGS(u32);

impl Default for C_CFLAGS {
    fn default() -> Self {
        let cbaud = C_CFLAGS_BAUD::B38400 as u32;
        let csize = C_CFLAGS_CSIZE::CS8 as u32;
        let c_cflags = cbaud | csize | CREAD;
        Self(c_cflags)
    }
}

impl C_CFLAGS {
    pub fn cbaud(&self) -> Result<C_CFLAGS_BAUD> {
        let cbaud = self.0 & CBAUD_MASK;
        Ok(C_CFLAGS_BAUD::try_from(cbaud)?)
    }

    pub fn csize(&self) -> Result<C_CFLAGS_CSIZE> {
        let csize = self.0 & CSIZE_MASK;
        Ok(C_CFLAGS_CSIZE::try_from(csize)?)
    }

    pub fn cread(&self) -> bool {
        self.0 & CREAD != 0
    }
}

const CREAD: u32 = 0x00000080;
const CBAUD_MASK: u32 = 0x0000100f;
const CSIZE_MASK: u32 = 0x00000030;

#[repr(u32)]
#[derive(Clone, Copy, TryFromInt)]
pub enum C_CFLAGS_CSIZE {
    CS5 = 0x00000000,
    CS6 = 0x00000010,
    CS7 = 0x00000020,
    CS8 = 0x00000030,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum C_CFLAGS_BAUD {
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
    #[repr(C)]
    #[derive(Pod)]
    pub struct C_LFLAGS: u32 {
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

impl Default for C_LFLAGS {
    fn default() -> Self {
        C_LFLAGS::ICANON
            | C_LFLAGS::ECHO
            | C_LFLAGS::ISIG
            | C_LFLAGS::ECHOE
            | C_LFLAGS::ECHOK
            | C_LFLAGS::ECHOCTL
            | C_LFLAGS::ECHOKE
            | C_LFLAGS::IEXTEN
    }
}

/* c_cc characters index*/
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum CC_C_CHAR {
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

impl CC_C_CHAR {
    // The special char is from gvisor
    pub fn default_char(&self) -> u8 {
        match self {
            CC_C_CHAR::VINTR => control_character('C'),
            CC_C_CHAR::VQUIT => control_character('\\'),
            CC_C_CHAR::VERASE => b'\x7f',
            CC_C_CHAR::VKILL => control_character('U'),
            CC_C_CHAR::VEOF => control_character('D'),
            CC_C_CHAR::VTIME => b'\0',
            CC_C_CHAR::VMIN => 1,
            CC_C_CHAR::VSWTC => b'\0',
            CC_C_CHAR::VSTART => control_character('Q'),
            CC_C_CHAR::VSTOP => control_character('S'),
            CC_C_CHAR::VSUSP => control_character('Z'),
            CC_C_CHAR::VEOL => b'\0',
            CC_C_CHAR::VREPRINT => control_character('R'),
            CC_C_CHAR::VDISCARD => control_character('O'),
            CC_C_CHAR::VWERASE => control_character('W'),
            CC_C_CHAR::VLNEXT => control_character('V'),
            CC_C_CHAR::VEOL2 => b'\0',
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct KernelTermios {
    c_iflags: C_IFLAGS,
    c_oflags: C_OFLAGS,
    c_cflags: C_CFLAGS,
    c_lflags: C_LFLAGS,
    c_line: CcT,
    c_cc: [CcT; KERNEL_NCCS],
}

impl Default for KernelTermios {
    fn default() -> Self {
        let mut termios = Self {
            c_iflags: C_IFLAGS::default(),
            c_oflags: C_OFLAGS::default(),
            c_cflags: C_CFLAGS::default(),
            c_lflags: C_LFLAGS::default(),
            c_line: 0,
            c_cc: [CcT::default(); KERNEL_NCCS],
        };
        *termios.get_special_char_mut(CC_C_CHAR::VINTR) = CC_C_CHAR::VINTR.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VQUIT) = CC_C_CHAR::VQUIT.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VERASE) = CC_C_CHAR::VERASE.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VKILL) = CC_C_CHAR::VKILL.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOF) = CC_C_CHAR::VEOF.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VTIME) = CC_C_CHAR::VTIME.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VMIN) = CC_C_CHAR::VMIN.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VSWTC) = CC_C_CHAR::VSWTC.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VSTART) = CC_C_CHAR::VSTART.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VSTOP) = CC_C_CHAR::VSTOP.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VSUSP) = CC_C_CHAR::VSUSP.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOL) = CC_C_CHAR::VEOL.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VREPRINT) = CC_C_CHAR::VREPRINT.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VDISCARD) = CC_C_CHAR::VDISCARD.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VWERASE) = CC_C_CHAR::VWERASE.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VLNEXT) = CC_C_CHAR::VLNEXT.default_char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOL2) = CC_C_CHAR::VEOL2.default_char();
        termios
    }
}

impl KernelTermios {
    pub fn get_special_char(&self, cc_c_char: CC_C_CHAR) -> &CcT {
        &self.c_cc[cc_c_char as usize]
    }

    pub fn get_special_char_mut(&mut self, cc_c_char: CC_C_CHAR) -> &mut CcT {
        &mut self.c_cc[cc_c_char as usize]
    }

    /// Canonical mode means we will handle input by lines, not by single character
    pub fn is_canonical_mode(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::ICANON)
    }

    /// ICRNL means we should map \r to \n
    pub fn contains_icrnl(&self) -> bool {
        self.c_iflags.contains(C_IFLAGS::ICRNL)
    }

    pub fn contains_isig(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::ISIG)
    }

    pub fn contain_echo(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::ECHO)
    }

    pub fn contains_echo_ctl(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::ECHOCTL)
    }

    pub fn contains_iexten(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::IEXTEN)
    }
}

const fn control_character(c: char) -> u8 {
    debug_assert!(c as u8 >= b'A');
    c as u8 - b'A' + 1u8
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}
