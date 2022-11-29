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

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct KernelTermios {
    pub c_iflags: C_IFLAGS,
    pub c_oflags: C_OFLAGS,
    pub c_cflags: C_CFLAGS,
    pub c_lflags: C_LFLAGS,
    pub c_line: CcT,
    pub c_cc: [CcT; KERNEL_NCCS],
}

impl KernelTermios {
    pub fn default() -> Self {
        Self {
            c_iflags: C_IFLAGS::ICRNL,
            c_oflags: C_OFLAGS::empty(),
            c_cflags: C_CFLAGS::B0,
            c_lflags: C_LFLAGS::ICANON | C_LFLAGS::ECHO,
            c_line: 0,
            c_cc: [0; KERNEL_NCCS],
        }
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

    pub fn is_cooked_mode(&self) -> bool {
        self.c_lflags.contains(C_LFLAGS::ICANON)
    }

    /// ICRNL means we should map \r to \n
    pub fn contains_icrnl(&self) -> bool {
        self.c_iflags.contains(C_IFLAGS::ICRNL)
    }
}
