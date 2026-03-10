// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::tty::CFontOp,
    util::ioctl::{InData, OutData, PassByVal, ioc},
};

// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/kd.h>

pub(super) type SetGraphicsMode = ioc!(KDSETMODE,  0x4B3A,     InData<i32, PassByVal>);
pub(super) type GetGraphicsMode = ioc!(KDGETMODE,  0x4B3B,     OutData<i32>);

pub(super) type GetKeyboardMode = ioc!(KDGKBMODE,  0x4B44,     OutData<i32>);
pub(super) type SetKeyboardMode = ioc!(KDSKBMODE,  0x4B45,     InData<i32, PassByVal>);

pub(super) type SetOrGetFont    = ioc!(KDFONTOP,   0x4B72,     InData<CFontOp>);
