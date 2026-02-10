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

/// The keyboard mode.
///
/// This mode determines how a console behaves when it receives input from the keyboard. For more
/// details, see <https://lct.sourceforge.net/lct/x60.html>.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/include/uapi/linux/kd.h#L81-L85>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(i32)]
pub enum KeyboardMode {
    /// The scancode mode (`K_RAW` in Linux).
    Raw = 0,
    /// The ASCII mode (`K_XLATE` in Linux).
    Xlate = 1,
    /// The keycode mode (`K_MEDIUMRAW` in Linux).
    MediumRaw = 2,
    /// The Unicode mode (`K_UNICODE` in Linux).
    Unicode = 3,
    /// The off mode (`K_OFF` in Linux).
    Off = 4,
}

bitflags::bitflags! {
    /// The keyboard mode flags.
    ///
    // Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/include/linux/kbd_kern.h#L53-L58>.
    pub struct KeyboardModeFlags: u8 {
        /// The application key mode (`VC_APPLIC` in Linux).
        const APPLICATION = 1 << 0;
        /// The cursor key mode (`VC_CKMODE` in Linux).
        const CURSOR_KEY = 1 << 1;
        /// The repeat mode (`VC_REPEAT` in Linux).
        const REPEAT = 1 << 2;
        /// The CRLF mode (`VC_CRLF` in Linux).
        ///
        /// If set, enter key sends `\r\n`; otherwise, it sends `\r` only.
        const CRLF   = 1 << 3;
        /// The meta key mode (`VC_META` in Linux).
        ///
        /// If set, every input character has a prefix with `ESC` (0x1B).
        const META   = 1 << 4;
    }
}
