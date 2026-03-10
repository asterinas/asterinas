// SPDX-License-Identifier: MPL-2.0

use crate::util::ioctl::{InData, NoData, OutData, ioc};

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

pub type SetPtyLock   = ioc!(TIOCSPTLCK,  b'T', 0x31, InData<i32>);
pub type GetPtyLock   = ioc!(TIOCGPTLCK,  b'T', 0x39, OutData<i32>);

pub type OpenPtySlave = ioc!(TIOCGPTPEER, b'T', 0x41, NoData);

pub type SetPktMode   = ioc!(TIOCPKT,     0x5420,     InData<i32>);
pub type GetPktMode   = ioc!(TIOCGPKT,    b'T', 0x38, OutData<i32>);

pub type GetPtyNumber = ioc!(TIOCGPTN, b'T', 0x30, OutData<u32>);
