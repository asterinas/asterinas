// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

/// An ioctl command that follows Linux-style encoding.
///
/// Layout (from LSB to MSB):
/// - bits 0..=7   : command number (nr)
/// - bits 8..=15  : type / magic
/// - bits 16..=29 : size (in bytes)
/// - bits 30..=31 : direction
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctl.h#L69-L73>
#[derive(Clone, Copy, Debug)]
pub(super) struct IoctlCmd(u32);

/// The direction of data transfer for an ioctl command.
///
/// The meaning is from the *user-space* perspective,
/// following Linux's `_IOC_*` macros:
///  - `None`  -> `_IOC_NONE`
///  - `Write` -> `_IOC_WRITE` (user to kernel)
///  - `Read`  -> `_IOC_READ`  (kernel to user)
///  - `ReadWrite` -> `_IOC_READ | _IOC_WRITE`
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctl.h#L49-L67>
#[derive(Clone, Copy, Debug, PartialEq, Eq, TryFromInt)]
#[repr(u8)]
pub enum IoctlDir {
    None = 0,
    Write = 1,
    Read = 2,
    ReadWrite = 3,
}

impl IoctlCmd {
    pub(super) const fn new(cmd: u32) -> Self {
        Self(cmd)
    }

    pub(super) const fn as_u32(self) -> u32 {
        self.0
    }

    pub(super) const fn nr(self) -> u8 {
        // Bits 0..=7
        self.0 as u8
    }

    pub(super) const fn set_nr(&mut self, nr: u8) {
        // Bits 0..=7
        self.0 = (self.0 & !0xFF) | (nr as u32);
    }

    pub(super) const fn magic(self) -> u8 {
        // Bits 8..=15
        (self.0 >> 8) as u8
    }

    pub(super) const fn size(self) -> u16 {
        // Bits 16..=29
        ((self.0 >> 16) as u16) & 0x3FFF
    }

    pub(super) fn dir(self) -> IoctlDir {
        // Bits 30..=31
        IoctlDir::try_from((self.0 >> 30) as u8).unwrap()
    }
}

pub trait DataSpec {
    const SIZE: Option<u16>;
    const DIR: IoctlDir;
}

pub trait PtrDataSpec: DataSpec {
    type Pointee;
}
