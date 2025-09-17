// SPDX-License-Identifier: MPL-2.0

//! Mode types.

use int_to_c_enum::TryFromInt;

/// The console mode (text or graphics).
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/include/uapi/linux/kd.h#L45>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(i32)]
pub enum ConsoleMode {
    /// The text mode (`KD_TEXT` in Linux). The console will display text characters.
    Text = 0,
    /// The graphics mode (`KD_GRAPHICS` in Linux). The console will not display text characters
    /// and may be used for graphical output (e.g., by X server).
    Graphics = 1,
}
