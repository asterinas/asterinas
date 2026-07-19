// SPDX-License-Identifier: MPL-2.0

/// C-compatible representation of `struct vt_stat`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L34-L38>
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
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
#[derive(Clone, Copy, Debug, Pod)]
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

/// Argument values for the `VT_RELDISP` ioctl.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L872-L881>
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleaseDisplayType {
    /// Deny a pending VT release request.
    DenyRelease = 0,
    /// Allow a pending VT release request.
    AllowRelease = 1,
    /// Acknowledge completion of VT acquisition.
    AckAcquire = 2,
}

impl From<i32> for ReleaseDisplayType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::DenyRelease,
            2 => Self::AckAcquire,
            _ => Self::AllowRelease,
        }
    }
}
