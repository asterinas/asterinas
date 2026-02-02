// SPDX-License-Identifier: MPL-2.0

use crate::util::ioctl::{OutData, ioc};

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

pub type GetNumBytesToRead = ioc!(FIONREAD, 0x541B, OutData<i32>);
