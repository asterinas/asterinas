// SPDX-License-Identifier: MPL-2.0

use super::termio::{CTermio, CTermios, CTermios2, CWinSize};
use crate::util::ioctl::{InData, OutData, ioc};

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

pub(crate) type GetTermios       = ioc!(TCGETS,     0x5401,     OutData<CTermios>);
pub(crate) type SetTermios       = ioc!(TCSETS,     0x5402,     InData<CTermios>);
pub(crate) type SetTermiosWait   = ioc!(TCSETSW,    0x5403,     InData<CTermios>);
pub(crate) type SetTermiosFlush  = ioc!(TCSETSF,    0x5404,     InData<CTermios>);
pub(crate) type GetTermio        = ioc!(TCGETA,     0x5405,     OutData<CTermio>);
pub(crate) type SetTermio        = ioc!(TCSETA,     0x5406,     InData<CTermio>);
pub(crate) type SetTermioWait    = ioc!(TCSETAW,    0x5407,     InData<CTermio>);
pub(crate) type SetTermioFlush   = ioc!(TCSETAF,    0x5408,     InData<CTermio>);

pub(crate) type GetTermios2      = ioc!(TCGETS2,    b'T', 0x2A, OutData<CTermios2>);
pub(crate) type SetTermios2      = ioc!(TCSETS2,    b'T', 0x2B, InData<CTermios2>);
pub(crate) type SetTermios2Wait  = ioc!(TCSETSW2,   b'T', 0x2C, InData<CTermios2>);
pub(crate) type SetTermios2Flush = ioc!(TCSETSF2,   b'T', 0x2D, InData<CTermios2>);

pub(crate) type GetWinSize       = ioc!(TIOCGWINSZ, 0x5413,     OutData<CWinSize>);
pub(crate) type SetWinSize       = ioc!(TIOCSWINSZ, 0x5414,     InData<CWinSize>);
