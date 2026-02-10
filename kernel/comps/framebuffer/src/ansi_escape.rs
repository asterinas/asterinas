// SPDX-License-Identifier: MPL-2.0

use crate::Pixel;

/// A finite-state machine (FSM) to handle ANSI escape sequences.
#[derive(Debug)]
pub struct EscapeFsm {
    state: WaitFor,
    params: [Option<u32>; MAX_PARAMS],
}

impl Default for EscapeFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// The mode for "Erase in Display" (ED) command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraseInDisplay {
    CursorToEnd,
    CursorToBeginning,
    EntireScreen,
    EntireScreenAndScrollback,
}

/// A trait to execute operations from ANSI escape sequences.
pub trait EscapeOp {
    /// Sets the cursor position.
    fn set_cursor(&mut self, x: usize, y: usize);

    /// Sets the foreground color.
    fn set_fg_color(&mut self, val: Pixel);
    /// Sets the background color.
    fn set_bg_color(&mut self, val: Pixel);

    /// Erases part or all of the display.
    fn erase_in_display(&mut self, mode: EraseInDisplay);
}

const MAX_PARAMS: usize = 8;

// FIXME: Currently we only support a few ANSI escape sequences, and we just swallow the unsupported ones.
#[derive(Clone, Copy, Debug)]
enum WaitFor {
    Escape,
    /// Just saw ESC (0x1B), expecting the next selector.
    AfterEscape,
    /// Currently parsing CSI parameters.
    Csi {
        idx: u8,
        is_private: bool,
        saw_digit: bool,
        in_intermediate: bool,
    },
    /// OSC payload after ESC] . Swallow until BEL (0x07) or ST (ESC \).
    Osc,
    /// Saw ESC inside OSC and it maybe ST.
    OscEscape,
}

/// Foreground and background colors.
///
/// See <https://en.wikipedia.org/wiki/ANSI_escape_code#3-bit_and_4-bit>.
#[rustfmt::skip]
const COLORS: [Pixel; 16] = [
    // Black
    Pixel { red: 0, green: 0, blue: 0 },
    // Red
    Pixel { red: 170, green: 0, blue: 0 },
    // Green
    Pixel { red: 0, green: 170, blue: 0 },
    // Yellow
    Pixel { red: 170, green: 85, blue: 0 },
    // Blue
    Pixel { red: 0, green: 0, blue: 170 },
    // Magenta
    Pixel { red: 170, green: 0, blue: 170 },
    // Cyan
    Pixel { red: 0, green: 170, blue: 170 },
    // White
    Pixel { red: 170, green: 170, blue: 170 },
    // Bright Black (Gray)
    Pixel { red: 85, green: 85, blue: 85 },
    // Bright Red
    Pixel { red: 255, green: 85, blue: 85 },
    // Bright Green
    Pixel { red: 85, green: 255, blue: 85 },
    // Bright Yellow
    Pixel { red: 255, green: 255, blue: 85 },
    // Bright Blue
    Pixel { red: 85, green: 85, blue: 255 },
    // Bright Magenta
    Pixel { red: 255, green: 85, blue: 255 },
    // Bright Cyan
    Pixel { red: 85, green: 255, blue: 255 },
    // Bright White
    Pixel { red: 255, green: 255, blue: 255 },
];

impl EscapeFsm {
    pub fn new() -> Self {
        Self {
            state: WaitFor::Escape,
            params: [None; MAX_PARAMS],
        }
    }

    /// Tries to eat a byte as part of the ANSI escape sequence.
    ///
    /// Returns `true` if the byte is consumed by the FSM and must not be rendered as text.
    /// Returns `false` if the byte is not part of an escape sequence and should be rendered.
    pub fn eat<T: EscapeOp>(&mut self, byte: u8, op: &mut T) -> bool {
        match self.state {
            WaitFor::Escape => {
                if byte == 0x1b {
                    self.state = WaitFor::AfterEscape;
                    return true;
                }
                false
            }

            WaitFor::AfterEscape => {
                match byte {
                    b'[' => {
                        // CSI begins.
                        self.params.fill(None);
                        self.state = WaitFor::Csi {
                            idx: 0,
                            is_private: false,
                            saw_digit: false,
                            in_intermediate: false,
                        };
                        true
                    }
                    b']' => {
                        // OSC begins.
                        self.state = WaitFor::Osc;
                        true
                    }
                    _ => {
                        // The character is invalid. We cannot handle it, so we are aborting the ANSI
                        // escape sequence.
                        self.state = WaitFor::Escape;
                        true
                    }
                }
            }

            WaitFor::Osc => {
                match byte {
                    0x07 => {
                        // BEL terminator
                        self.state = WaitFor::Escape;
                        true
                    }
                    0x1b => {
                        // Might be ST
                        self.state = WaitFor::OscEscape;
                        true
                    }
                    _ => {
                        // Swallow OSC payload.
                        true
                    }
                }
            }

            WaitFor::OscEscape => {
                if byte == b'\\' {
                    self.state = WaitFor::Escape;
                    true
                } else {
                    // Not ST and we go back to OSC and keep swallowing.
                    self.state = WaitFor::Osc;
                    true
                }
            }

            WaitFor::Csi {
                idx,
                is_private,
                saw_digit,
                in_intermediate,
            } => {
                match byte {
                    // Intermediate bytes (0x20..=0x2F).
                    // Once we see any intermediate, we are in the intermediate section;
                    // later bytes must be intermediate or final.
                    0x20..=0x2f => {
                        // If we already entered intermediate section, just keep swallowing them.
                        // If we were still in parameter section, we now transition to intermediate.
                        self.state = WaitFor::Csi {
                            idx,
                            is_private,
                            saw_digit,
                            in_intermediate: true,
                        };
                        true
                    }

                    // Parameter bytes (0x30..=0x3F).
                    0x30..=0x3f if !in_intermediate => {
                        match byte {
                            // digits: contribute to numeric params
                            b'0'..=b'9' => {
                                let i = idx as usize;
                                if i < MAX_PARAMS {
                                    let p = &mut self.params[i];
                                    *p = Some(
                                        p.unwrap_or(0)
                                            .saturating_mul(10)
                                            .saturating_add((byte - b'0') as u32),
                                    );
                                }
                                self.state = WaitFor::Csi {
                                    idx,
                                    is_private,
                                    saw_digit: true,
                                    in_intermediate: false,
                                };
                            }

                            // ';' separates numeric parameters.
                            b';' => {
                                let next = idx.saturating_add(1);
                                if (next as usize) < MAX_PARAMS {
                                    // If there were no digits for this param, it already stays None.
                                    self.state = WaitFor::Csi {
                                        idx: next,
                                        is_private,
                                        saw_digit: false,
                                        in_intermediate: false,
                                    };
                                } else {
                                    // There are too many parameters. We cannot handle that many, so we are aborting
                                    // the ANSI escape sequence.
                                    self.state = WaitFor::Escape;
                                }
                            }

                            b':' => {
                                // The behavior of ':' is not defined by the standard.
                                log::warn!("EscapeFsm: unsupported ':' parameter separator in CSI");
                            }

                            // Sequences containing <=>? are "private". We swallow them and mark `is_private`.
                            b'<' | b'=' | b'>' | b'?' => {
                                self.state = WaitFor::Csi {
                                    idx,
                                    is_private: true,
                                    saw_digit,
                                    in_intermediate: false,
                                };
                            }

                            _ => unreachable!(),
                        }
                        true
                    }

                    // Parameter bytes after intermediate section is illegal by the formal grammar.
                    // We'll abort and swallow to avoid leaking garbage.
                    0x30..=0x3f if in_intermediate => {
                        self.state = WaitFor::Escape;
                        true
                    }

                    // Final byte (0x40..=0x7E): ends the CSI.
                    0x40..=0x7e => {
                        self.state = WaitFor::Escape;

                        let num_params = (idx as usize).saturating_add(1).min(MAX_PARAMS);

                        self.dispatch_csi(byte, num_params, is_private, op);
                        true
                    }

                    _ => {
                        // Terminal behavior is undefined if a CSI contains bytes outside 0x20..=0x7E.
                        log::warn!("EscapeFsm: invalid byte {:#x} in CSI sequence", byte);
                        self.state = WaitFor::Escape;
                        true
                    }
                }
            }
        }
    }

    /// Gets the parameter at the given index, or returns the default value if the parameter is not present.
    fn param_or(&self, i: usize, default: u32) -> u32 {
        self.params.get(i).and_then(|p| *p).unwrap_or(default)
    }

    fn dispatch_csi<T: EscapeOp>(
        &self,
        final_byte: u8,
        num_params: usize,
        is_private: bool,
        op: &mut T,
    ) {
        if is_private {
            // For now we don't handle any private sequences, so just swallow them.
            return;
        }

        match final_byte {
            // CUP - Cursor Position: CSI n ; m H
            //
            // - n=row, m=col
            // - default to 1 if omitted
            //
            // Examples:
            // - CSI H        -> 1;1
            // - CSI ;5H      -> 1;5
            // - CSI 17H      -> 17;1
            // - CSI 17;H     -> 17;1
            // - CSI 17;1H    -> 17;1
            b'H' => {
                let row_1b = self.param_or(0, 1);
                let col_1b = self.param_or(1, 1);

                op.set_cursor((col_1b - 1) as usize, (row_1b - 1) as usize);
            }

            // ED - Erase in Display: CSI n J
            //
            // n:
            // - 0 (or missing): cursor to end of screen
            // - 1: cursor to beginning of screen
            // - 2: entire screen
            // - 3: entire screen + scrollback
            b'J' => {
                let n = self.param_or(0, 0);
                let mode = match n {
                    0 => EraseInDisplay::CursorToEnd,
                    1 => EraseInDisplay::CursorToBeginning,
                    2 => EraseInDisplay::EntireScreen,
                    3 => EraseInDisplay::EntireScreenAndScrollback,
                    _ => {
                        // Invalid parameter.
                        return;
                    }
                };
                op.erase_in_display(mode);
            }

            // SGR - Select Graphic Rendition
            b'm' => self.handle_sgr(num_params, op),

            // Unknown CSI: swallow silently.
            _ => {}
        }
    }

    /// Handles the "Select Graphic Rendition" sequence.
    fn handle_sgr<T: EscapeOp>(&self, num_params: usize, op: &mut T) {
        let mut cursor = 0;
        while cursor < num_params {
            let op_code = self.param_or(cursor, 0) as u8;
            cursor += 1;

            match op_code {
                // Reset text attributes
                0 => {
                    op.set_fg_color(Pixel::WHITE);
                    op.set_bg_color(Pixel::BLACK);
                }

                // Set foreground colors
                // Reference: <https://en.wikipedia.org/wiki/ANSI_escape_code#Colors>
                30..=37 => op.set_fg_color(COLORS[op_code as usize - 30]),
                38 if num_params - cursor >= 2 && self.param_or(cursor, 0) == 5 => {
                    op.set_fg_color(Self::get_256_color(self.param_or(cursor + 1, 0) as u8));
                    cursor += 2;
                }
                38 if num_params - cursor >= 4 && self.param_or(cursor, 0) == 2 => {
                    op.set_fg_color(Pixel {
                        red: self.param_or(cursor + 1, 0) as u8,
                        green: self.param_or(cursor + 2, 0) as u8,
                        blue: self.param_or(cursor + 3, 0) as u8,
                    });
                    cursor += 4;
                }
                // Reset to the default foreground color
                39 => op.set_fg_color(Pixel::WHITE),
                90..=97 => op.set_fg_color(COLORS[op_code as usize - 90 + 8]),

                // Set background colors
                // Reference: <https://en.wikipedia.org/wiki/ANSI_escape_code#Colors>
                40..=47 => op.set_bg_color(COLORS[op_code as usize - 40]),
                48 if num_params - cursor >= 2 && self.param_or(cursor, 0) == 5 => {
                    op.set_bg_color(Self::get_256_color(self.param_or(cursor + 1, 0) as u8));
                    cursor += 2;
                }
                48 if num_params - cursor >= 4 && self.param_or(cursor, 0) == 2 => {
                    op.set_bg_color(Pixel {
                        red: self.param_or(cursor + 1, 0) as u8,
                        green: self.param_or(cursor + 2, 0) as u8,
                        blue: self.param_or(cursor + 3, 0) as u8,
                    });
                    cursor += 4;
                }
                // Reset to the default background color
                49 => op.set_bg_color(Pixel::BLACK),
                100..=107 => op.set_bg_color(COLORS[op_code as usize - 100 + 8]),

                // Invalid or unsupported
                _ => return,
            }
        }
    }

    /// Gets the 256-color used by Linux TTY.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.7/source/drivers/tty/vt/vt.c#L1599>
    fn get_256_color(i: u8) -> Pixel {
        // Arithmetic operations below won't overflow because `i` comes from a `u8`.
        let mut i = i as u16;
        let (red, green, blue) = match i {
            // Standard colors.
            0..=7 => {
                let r = if i & 1 != 0 { 0xaa } else { 0x00 };
                let g = if i & 2 != 0 { 0xaa } else { 0x00 };
                let b = if i & 4 != 0 { 0xaa } else { 0x00 };
                (r, g, b)
            }
            8..=15 => {
                let r = if i & 1 != 0 { 0xff } else { 0x55 };
                let g = if i & 2 != 0 { 0xff } else { 0x55 };
                let b = if i & 4 != 0 { 0xff } else { 0x55 };
                (r, g, b)
            }
            // 6x6x6 color cube.
            16..=231 => {
                i -= 16;
                let b = i % 6 * 255 / 6;
                i /= 6;
                let g = i % 6 * 255 / 6;
                i /= 6;
                let r = i * 255 / 6;
                (r as u8, g as u8, b as u8)
            }
            // Grayscale ramp.
            _ => {
                let g = (i * 10 - 2312) as u8;
                (g, g, g)
            }
        };

        Pixel { red, green, blue }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    struct State {
        x: usize,
        y: usize,
        fg: Pixel,
        bg: Pixel,
        last_ed: Option<EraseInDisplay>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                x: 0,
                y: 0,
                fg: Pixel::WHITE,
                bg: Pixel::BLACK,
                last_ed: None,
            }
        }
    }

    impl EscapeOp for State {
        fn set_cursor(&mut self, x: usize, y: usize) {
            self.x = x;
            self.y = y;
        }

        fn set_fg_color(&mut self, val: Pixel) {
            self.fg = val;
        }

        fn set_bg_color(&mut self, val: Pixel) {
            self.bg = val;
        }

        fn erase_in_display(&mut self, mode: EraseInDisplay) {
            self.last_ed = Some(mode);
        }
    }

    fn eat_escape_sequence(esc_fsm: &mut EscapeFsm, state: &mut State, bytes: &[u8]) {
        for byte in bytes {
            assert!(esc_fsm.eat(*byte, state));
        }
    }

    #[ktest]
    fn move_cursor() {
        let mut esc_fsm = EscapeFsm::new();
        let mut state = State::default();

        // Move the cursor to the third row (y=2) and the second column (x=1).
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[3;2H");
        assert_eq!(state.x, 1);
        assert_eq!(state.y, 2);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // CUP defaults to 1;1 when omitted.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[H");
        assert_eq!(state.x, 0);
        assert_eq!(state.y, 0);

        assert!(!esc_fsm.eat(b'a', &mut state));
    }

    #[ktest]
    fn erase_in_display() {
        let mut esc_fsm = EscapeFsm::new();
        let mut state = State::default();

        // Default (or missing) is 0: cursor to end of screen.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[J");
        assert_eq!(state.last_ed, Some(EraseInDisplay::CursorToEnd));

        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[1J");
        assert_eq!(state.last_ed, Some(EraseInDisplay::CursorToBeginning));

        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[2J");
        assert_eq!(state.last_ed, Some(EraseInDisplay::EntireScreen));

        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[3J");
        assert_eq!(
            state.last_ed,
            Some(EraseInDisplay::EntireScreenAndScrollback)
        );

        assert!(!esc_fsm.eat(b'a', &mut state));
    }

    #[ktest]
    fn set_color() {
        let mut esc_fsm = EscapeFsm::new();
        let mut state = State::default();

        // Set the foreground color and background color to "Black".
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[30;40m");
        assert_eq!(state.fg, Pixel::BLACK);
        assert_eq!(state.bg, Pixel::BLACK);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // Reset.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[0m");
        assert_eq!(state.fg, Pixel::WHITE);
        assert_eq!(state.bg, Pixel::BLACK);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // Set the foreground color and background color to "Bright White".
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[97m");
        assert_eq!(state.fg, Pixel::WHITE);
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[107m");
        assert_eq!(state.bg, Pixel::WHITE);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // Reset.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[m");
        assert_eq!(state.fg, Pixel::WHITE);
        assert_eq!(state.bg, Pixel::BLACK);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // Set the foreground color and background color using 8-bit/24-bit code.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[38;5;0m");
        assert_eq!(state.fg, Pixel::BLACK);
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[48;2;255;255;255m");
        assert_eq!(state.bg, Pixel::WHITE);
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[38;5;8;48;5;16m");
        assert_eq!(
            state.fg,
            Pixel {
                red: 85,
                green: 85,
                blue: 85
            }
        );
        assert_eq!(state.bg, Pixel::BLACK);
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[39;48;2;41;41;41m");
        assert_eq!(state.fg, Pixel::WHITE);
        assert_eq!(
            state.bg,
            Pixel {
                red: 41,
                green: 41,
                blue: 41
            }
        );
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[38;5;255;49m");
        assert_eq!(
            state.fg,
            Pixel {
                red: 238,
                green: 238,
                blue: 238
            }
        );
        assert_eq!(state.bg, Pixel::BLACK);

        assert!(!esc_fsm.eat(b'a', &mut state));

        // Reset.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[m");
        assert_eq!(state.fg, Pixel::WHITE);
        assert_eq!(state.bg, Pixel::BLACK);

        assert!(!esc_fsm.eat(b'a', &mut state));
    }
}
