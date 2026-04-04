// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::tty::{
        CFontOp,
        vt::c_types::{CVtMode, CVtState},
    },
    util::ioctl::{InData, OutData, PassByVal, ioc},
};

// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/kd.h>

/// Gets the keyboard type.
pub(super) type GetKeyboardType = ioc!(KDGKBTYPE,   0x4B33,     OutData<u8>);

/// Gets the display mode.
pub(super) type GetGraphicsMode = ioc!(KDGETMODE,   0x4B3B,     OutData<i32>);
/// Sets the display mode.
pub(super) type SetGraphicsMode = ioc!(KDSETMODE,   0x4B3A,     InData<i32, PassByVal>);

/// Gets the keyboard mode.
pub(super) type GetKeyboardMode = ioc!(KDGKBMODE,   0x4B44,     OutData<i32>);
/// Sets the keyboard mode.
pub(super) type SetKeyboardMode = ioc!(KDSKBMODE,   0x4B45,     InData<i32, PassByVal>);

/// Sets or gets font data.
pub(super) type SetOrGetFont    = ioc!(KDFONTOP,    0x4B72,     InData<CFontOp>);

// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h>

/// Returns the first available (non-opened) VT number.
/// If all VTs are in use, returns -1.
///
/// Valid VT numbers start from 1.
pub(super) type GetAvailableVt  = ioc!(VT_OPENQRY,  0x5600,     OutData<i32>);

/// Gets the VT mode.
pub(super) type GetVtMode       = ioc!(VT_GETMODE,  0x5601,     OutData<CVtMode>);
/// Sets the VT mode.
pub(super) type SetVtMode       = ioc!(VT_SETMODE,  0x5602,     InData<CVtMode>);

/// Gets the global VT state.
///
/// Note:
/// - VT 0 is always open (alias for active VT).
/// - At most 16 VT states can be returned due to ABI constraints.
pub(super) type GetVtState      = ioc!(VT_GETSTATE, 0x5603,     OutData<CVtState>);

/// Used in process-controlled VT switching to allow or deny
/// VT release, or to acknowledge VT acquisition.
pub(super) type ReleaseDisplay  = ioc!(VT_RELDISP,  0x5605,     InData<i32, PassByVal>);

/// Activates the specified VT (VT numbers start from 1).
///
/// Switching to VT 0 is not allowed.
pub(super) type ActivateVt      = ioc!(VT_ACTIVATE,  0x5606,    InData<i32, PassByVal>);
/// Blocks until the specified VT becomes active.
pub(super) type WaitForVtActive = ioc!(VT_WAITACTIVE,   0x5607,     InData<i32, PassByVal>);

/// Disallocates the specified VT if argument is non-zero, or releases all VTs if argument
/// is zero. But we don't disallocate VT 1 since it's the default VT and always open.
pub(super) type DisallocateVt   = ioc!(VT_DISALLOCATE,  0x5608,     InData<i32, PassByVal>);
