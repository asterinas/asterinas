// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::tty::vt::console::{VtMode, VtModeType},
    prelude::{Errno, Result, TryFromInt},
    process::signal::sig_num::SigNum,
};

/// C-compatible representation of `struct vt_stat`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L34-L38>
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CVtState {
    /// Currently active VT number (starting from 1).
    pub active: u16,
    /// Signal number to be sent on VT switch.
    pub signal: u16,
    /// Bitmask representing VT state.
    pub state: u16,
}

/// C-compatible representation of `struct vt_mode`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L21-L27>
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CVtMode {
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
        let release_signal = if self.relsig == 0 {
            None
        } else {
            Some(SigNum::try_from(self.relsig as u8)?)
        };
        let acquire_signal = if self.acqsig == 0 {
            None
        } else {
            Some(SigNum::try_from(self.acqsig as u8)?)
        };

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
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L872-L881>
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub(super) enum ReleaseDisplayType {
    /// Deny a pending VT release request.
    DenyRelease = 0,
    /// Allow a pending VT release request.
    AllowRelease = 1,
    /// Acknowledge completion of VT acquisition.
    AckAcquire = 2,
}
