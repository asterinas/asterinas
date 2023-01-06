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
        const IGNBRK	= 0x001;			/* Ignore break condition */
        const BRKINT	= 0x002;			/* Signal interrupt on break */
        const IGNPAR	= 0x004;			/* Ignore characters with parity errors */
        const PARMRK	= 0x008;			/* Mark parity and framing errors */
        const INPCK	    = 0x010;			/* Enable input parity check */
        const ISTRIP	= 0x020;			/* Strip 8th bit off characters */
        const INLCR	    = 0x040;			/* Map NL to CR on input */
        const IGNCR	    = 0x080;			/* Ignore CR */
        const ICRNL	    = 0x100;			/* Map CR to NL on input */
        const IXANY	    = 0x800;			/* Any character will restart after stop */
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/termbits.h
        const IUCLC	    = 0x0200;
        const IXON	    = 0x0400;
        const IXOFF	    = 0x1000;
        const IMAXBEL	= 0x2000;
        const IUTF8	    = 0x4000;
    }
}

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct C_OFLAGS: u32 {
        const OPOST	= 0x01;			/* Perform output processing */
        const OCRNL	= 0x08;
        const ONOCR	= 0x10;
        const ONLRET= 0x20;
        const OFILL	= 0x40;
        const OFDEL	= 0x80;
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, Pod)]
pub enum C_CFLAGS {
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
        const ISIG	=   0x00001;
        const ICANON=   0x00002;
        const XCASE	=   0x00004;
        const ECHO	=   0x00008;
        const ECHOE	=   0x00010;
        const ECHOK	=   0x00020;
        const ECHONL=	0x00040;
        const NOFLSH=	0x00080;
        const TOSTOP=	0x00100;
        const ECHOCTL=	0x00200;
        const ECHOPRT=	0x00400;
        const ECHOKE=   0x00800;
        const FLUSHO=	0x01000;
        const PENDIN=	0x04000;
        const IEXTEN=	0x08000;
        const EXTPROC=	0x10000;
    }
}

/* c_cc characters index*/
#[repr(u32)]
#[derive(Debug, Clone, Copy, Pod)]
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
    // The special char is the same as ubuntu
    pub fn char(&self) -> u8 {
        match self {
            CC_C_CHAR::VINTR => 3,
            CC_C_CHAR::VQUIT => 28,
            CC_C_CHAR::VERASE => 127,
            CC_C_CHAR::VKILL => 21,
            CC_C_CHAR::VEOF => 4,
            CC_C_CHAR::VTIME => 0,
            CC_C_CHAR::VMIN => 1,
            CC_C_CHAR::VSWTC => 0,
            CC_C_CHAR::VSTART => 17,
            CC_C_CHAR::VSTOP => 19,
            CC_C_CHAR::VSUSP => 26,
            CC_C_CHAR::VEOL => 255,
            CC_C_CHAR::VREPRINT => 18,
            CC_C_CHAR::VDISCARD => 15,
            CC_C_CHAR::VWERASE => 23,
            CC_C_CHAR::VLNEXT => 22,
            CC_C_CHAR::VEOL2 => 255,
        }
    }

    pub fn as_usize(&self) -> usize {
        *self as usize
    }

    pub fn from_char(item: u8) -> Result<Self> {
        if item == Self::VINTR.char() {
            return Ok(Self::VINTR);
        }
        if item == Self::VQUIT.char() {
            return Ok(Self::VQUIT);
        }
        if item == Self::VINTR.char() {
            return Ok(Self::VINTR);
        }
        if item == Self::VERASE.char() {
            return Ok(Self::VERASE);
        }
        if item == Self::VEOF.char() {
            return Ok(Self::VEOF);
        }
        if item == Self::VSTART.char() {
            return Ok(Self::VSTART);
        }
        if item == Self::VSTOP.char() {
            return Ok(Self::VSTOP);
        }
        if item == Self::VSUSP.char() {
            return Ok(Self::VSUSP);
        }

        return_errno_with_message!(Errno::EINVAL, "Not a valid cc_char");
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

impl KernelTermios {
    pub fn default() -> Self {
        let mut termios = Self {
            c_iflags: C_IFLAGS::ICRNL,
            c_oflags: C_OFLAGS::empty(),
            c_cflags: C_CFLAGS::B0,
            c_lflags: C_LFLAGS::ICANON | C_LFLAGS::ECHO,
            c_line: 0,
            c_cc: [0; KERNEL_NCCS],
        };
        *termios.get_special_char_mut(CC_C_CHAR::VINTR) = CC_C_CHAR::VINTR.char();
        *termios.get_special_char_mut(CC_C_CHAR::VQUIT) = CC_C_CHAR::VQUIT.char();
        *termios.get_special_char_mut(CC_C_CHAR::VERASE) = CC_C_CHAR::VERASE.char();
        *termios.get_special_char_mut(CC_C_CHAR::VKILL) = CC_C_CHAR::VKILL.char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOF) = CC_C_CHAR::VEOF.char();
        *termios.get_special_char_mut(CC_C_CHAR::VTIME) = CC_C_CHAR::VTIME.char();
        *termios.get_special_char_mut(CC_C_CHAR::VMIN) = CC_C_CHAR::VMIN.char();
        *termios.get_special_char_mut(CC_C_CHAR::VSWTC) = CC_C_CHAR::VSWTC.char();
        *termios.get_special_char_mut(CC_C_CHAR::VSTART) = CC_C_CHAR::VSTART.char();
        *termios.get_special_char_mut(CC_C_CHAR::VSTOP) = CC_C_CHAR::VSTOP.char();
        *termios.get_special_char_mut(CC_C_CHAR::VSUSP) = CC_C_CHAR::VSUSP.char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOL) = CC_C_CHAR::VEOL.char();
        *termios.get_special_char_mut(CC_C_CHAR::VREPRINT) = CC_C_CHAR::VREPRINT.char();
        *termios.get_special_char_mut(CC_C_CHAR::VDISCARD) = CC_C_CHAR::VDISCARD.char();
        *termios.get_special_char_mut(CC_C_CHAR::VWERASE) = CC_C_CHAR::VWERASE.char();
        *termios.get_special_char_mut(CC_C_CHAR::VLNEXT) = CC_C_CHAR::VLNEXT.char();
        *termios.get_special_char_mut(CC_C_CHAR::VEOL2) = CC_C_CHAR::VEOL2.char();
        termios
    }

    fn new() -> Self {
        KernelTermios {
            c_iflags: C_IFLAGS::empty(),
            c_oflags: C_OFLAGS::empty(),
            c_cflags: C_CFLAGS::B0,
            c_lflags: C_LFLAGS::empty(),
            c_line: 0,
            c_cc: [0; KERNEL_NCCS],
        }
    }

    pub fn get_special_char(&self, cc_c_char: CC_C_CHAR) -> &CcT {
        &self.c_cc[cc_c_char.as_usize()]
    }

    pub fn get_special_char_mut(&mut self, cc_c_char: CC_C_CHAR) -> &mut CcT {
        &mut self.c_cc[cc_c_char.as_usize()]
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
