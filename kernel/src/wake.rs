// SPDX-License-Identifier: MPL-2.0

//! This module defines the wake up flags used by the Asterinas kernel.

use ostd::sync::WakeFlag;

pub const WAKE_DEFAULT: WakeFlag = WakeFlag::Flag0;
pub const WAKE_TIMEOUT: WakeFlag = WakeFlag::Flag1;
pub const WAKE_SIGNAL: WakeFlag = WakeFlag::Flag2;
