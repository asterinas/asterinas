// SPDX-License-Identifier: MPL-2.0

//! Common `ioctl` command definitions.
//!
//! This module defines `ioctl` commands that are widely supported across various
//! file and device types.

use crate::{ioc, util::ioctl::OutData};

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

pub(crate) type GetNumBytesToRead = ioc!(FIONREAD, 0x541B, OutData<i32>);
