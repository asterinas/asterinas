// SPDX-License-Identifier: MPL-2.0

use super::{
    CFontOp,
    termio::{CTermios, CWinSize},
};
use crate::util::ioctl::{InData, OutData, ioc};

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

pub type GetTermios      = ioc!(TCGETS,     0x5401,     OutData<CTermios>);
pub type SetTermios      = ioc!(TCSETS,     0x5402,     InData<CTermios>);
pub type SetTermiosWait  = ioc!(TCSETSW,    0x5403,     InData<CTermios>);
pub type SetTermiosFlush = ioc!(TCSETSF,    0x5404,     InData<CTermios>);

pub type GetWinSize      = ioc!(TIOCGWINSZ, 0x5413,     OutData<CWinSize>);
pub type SetWinSize      = ioc!(TIOCSWINSZ, 0x5414,     InData<CWinSize>);

// TODO: Consider moving this to the `pty` module.
pub type GetPtyNumber    = ioc!(TIOCGPTN,   b'T', 0x30, OutData<u32>);

pub type SetOrGetFont    = ioc!(KDFONTOP,   0x4B72,     InData<CFontOp>);
