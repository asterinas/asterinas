// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::tty::vt::console::{VtMode, VtModeType},
    prelude::{Errno, Result, TryFromInt},
    process::signal::sig_num::SigNum,
    util::ioctl::{InData, OutData, PassByVal, ioc},
};

/// C-compatible representation of `struct vt_stat`.
///
/// References: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L34-L38>
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CVtState {
    /// Currently active VT number (starting from 1).
    pub active: u16,
    /// Signal number to be sent on VT switch.
    pub signal: u16,
    /// Bitmask representing VT state.
    pub state: u16,
}

/// C-compatible representation of `struct vt_mode`.
///
// References: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L21-L27>
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CVtMode {
    /// VT mode type.
    pub mode: u8,
    /// If non-zero, writes block while this VT is inactive.
    pub waitv: u8,
    /// Signal sent when this VT is released.
    pub relsig: u16,
    /// Signal sent when this VT is acquired.
    pub acqsig: u16,
    /// Unused field. Must be set to 0.
    pub frsig: u16,
}

impl From<VtMode> for CVtMode {
    fn from(mode: VtMode) -> Self {
        CVtMode {
            mode: mode.mode_type as u8,
            waitv: mode.wait_on_inactive as u8,
            relsig: mode.release_signal.map_or(0, |s| s.as_u8() as u16),
            acqsig: mode.acquire_signal.map_or(0, |s| s.as_u8() as u16),
            frsig: 0,
        }
    }
}

impl TryInto<VtMode> for CVtMode {
    type Error = crate::prelude::Error;

    fn try_into(self) -> Result<VtMode> {
        let mode_type = VtModeType::try_from(self.mode)
            .map_err(|_| Self::Error::with_message(Errno::EINVAL, "invalid VT mode type"))?;
        let wait_on_inactive = self.waitv != 0;
        let release_signal = SigNum::try_from(self.relsig as u8).map(Some)?;
        let acquire_signal = SigNum::try_from(self.acqsig as u8).map(Some)?;

        Ok(VtMode {
            mode_type,
            wait_on_inactive,
            release_signal,
            acquire_signal,
        })
    }
}

/// Argument values for the `VT_RELDISP` ioctl.
///
// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L872-L881>
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub enum ReleaseDisplayType {
    /// Deny a pending VT release request.
    DenyRelease = 0,
    /// Allow a pending VT release request.
    AllowRelease = 1,
    /// Acknowledge completion of VT acquisition.
    AckAcquire = 2,
}

/// Returns the first available (non-opened) VT number.
/// If all VTs are in use, returns -1.
///
/// Valid VT numbers start from 1.
pub type GetAvailableVt = ioc!(VT_OPENQRY, 0x5600, OutData<i32>);

/// Get the VT mode.
pub type GetVtMode = ioc!(VT_GETMODE, 0x5601, OutData<CVtMode>);
/// Set the VT mode.
pub type SetVtMode = ioc!(VT_SETMODE, 0x5602, InData<CVtMode>);

/// Get the global VT state.
///
/// Note:
/// - VT 0 is always open (alias for active VT).
/// - At most 16 VT states can be returned due to ABI constraints.
pub type GetVtState = ioc!(VT_GETSTATE, 0x5603, OutData<CVtState>);

/// Activate the specified VT (VT numbers start from 1).
///
/// Switching to VT 0 is not allowed.
pub type ActivateVt = ioc!(VT_ACTIVATE, 0x5606, InData<i32, PassByVal>);
/// Block until the specified VT becomes active.
pub type WaitForVtActive = ioc!(VT_WAITACTIVE, 0x5607, InData<i32, PassByVal>);

/// Get the display mode.
pub type GetGraphicsMode = ioc!(KDGETMODE,  0x4B3B,     OutData<i32>);
/// Set the display mode.
pub type SetGraphicsMode = ioc!(KDSETMODE,  0x4B3A,     InData<i32, PassByVal>);

/// Get the keyboard mode.
pub type GetKeyboardMode = ioc!(KDGKBMODE,  0x4B44,     OutData<i32>);
/// Set the keyboard mode.
pub type SetKeyboardMode = ioc!(KDSKBMODE,  0x4B45,     InData<i32, PassByVal>);

/// Used in process-controlled VT switching to allow or deny
/// VT release, or to acknowledge VT acquisition.
pub type ReleaseDisplay = ioc!(VT_RELDISP,  0x5605, InData<i32, PassByVal>);
