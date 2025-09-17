// SPDX-License-Identifier: MPL-2.0

use crate::Pixel;

/// A finite-state machine (FSM) to handle ANSI escape sequences.
#[derive(Debug)]
pub(super) struct EscapeFsm {
    state: WaitFor,
    params: [u32; MAX_PARAMS],
}

/// A trait to execute operations from ANSI escape sequences.
pub(super) trait EscapeOp {
    /// Sets the cursor position.
    fn set_cursor(&mut self, x: usize, y: usize);

    /// Sets the foreground color.
    fn set_fg_color(&mut self, val: Pixel);
    /// Sets the background color.
    fn set_bg_color(&mut self, val: Pixel);
}

const MAX_PARAMS: usize = 8;

#[derive(Clone, Copy, Debug)]
enum WaitFor {
    Escape,
    Bracket,
    Params(u8),
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
    pub(super) fn new() -> Self {
        Self {
            state: WaitFor::Escape,
            params: [0; MAX_PARAMS],
        }
    }

    /// Tries to eat a character as part of the ANSI escape sequence.
    ///
    /// This method returns a boolean value indicating whether the character is part of an ANSI
    /// escape sequence. In other words, if the method returns true, then the character has been
    /// eaten and should not be displayed in the console.
    pub(super) fn eat<T: EscapeOp>(&mut self, byte: u8, op: &mut T) -> bool {
        let num_params = match (self.state, byte) {
            // Handle '\033'.
            (WaitFor::Escape, 0o33) => {
                self.state = WaitFor::Bracket;
                return true;
            }
            (WaitFor::Escape, _) => {
                // This is not an ANSI escape sequence.
                return false;
            }

            // Handle '['.
            (WaitFor::Bracket, b'[') => {
                self.state = WaitFor::Params(0);
                self.params[0] = 0;
                return true;
            }
            (WaitFor::Bracket, _) => {
                // The character is invalid. We cannot handle it, so we are aborting the ANSI
                // escape sequence.
                self.state = WaitFor::Escape;
                return true;
            }

            // Handle numeric parameters.
            (WaitFor::Params(i), b'0'..=b'9') => {
                let param = &mut self.params[i as usize];
                *param = param.wrapping_mul(10).wrapping_add((byte - b'0') as u32);
                return true;
            }
            (WaitFor::Params(i), b';') if (i as usize + 1) < MAX_PARAMS => {
                self.state = WaitFor::Params(i + 1);
                self.params[i as usize + 1] = 0;
                return true;
            }
            (WaitFor::Params(_), b';') => {
                // There are too many parameters. We cannot handle that many, so we are aborting
                // the ANSI escape sequence.
                self.state = WaitFor::Escape;
                return true;
            }

            // Break and handle the final action.
            (WaitFor::Params(i), _) => {
                self.state = WaitFor::Escape;
                (i + 1) as usize
            }
        };

        match byte {
            // CUP - Cursor Position
            b'H' if num_params == 2 => {
                op.set_cursor(
                    self.params[1].saturating_sub(1) as usize,
                    self.params[0].saturating_sub(1) as usize,
                );
            }

            // SGR - Select Graphic Rendition
            b'm' => self.handle_srg(num_params, op),

            // Invalid or unsupported
            _ => {}
        }

        true
    }

    /// Handles the "Select Graphic Rendition" sequence.
    fn handle_srg<T: EscapeOp>(&self, num_params: usize, op: &mut T) {
        let mut cursor = 0;
        while cursor < num_params {
            let op_code = self.params[cursor];
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
                38 if num_params - cursor >= 2 && self.params[cursor] == 5 => {
                    op.set_fg_color(Self::get_256_color(self.params[cursor + 1] as u8));
                    cursor += 2;
                }
                38 if num_params - cursor >= 4 && self.params[cursor] == 2 => {
                    op.set_fg_color(Pixel {
                        red: self.params[cursor + 1] as u8,
                        green: self.params[cursor + 2] as u8,
                        blue: self.params[cursor + 3] as u8,
                    });
                    cursor += 4;
                }
                // Reset to the default foreground color
                39 => op.set_fg_color(Pixel::WHITE),
                90..=97 => op.set_fg_color(COLORS[op_code as usize - 90 + 8]),

                // Set background colors
                // Reference: <https://en.wikipedia.org/wiki/ANSI_escape_code#Colors>
                40..=47 => op.set_bg_color(COLORS[op_code as usize - 40]),
                48 if num_params - cursor >= 2 && self.params[cursor] == 5 => {
                    op.set_bg_color(Self::get_256_color(self.params[cursor + 1] as u8));
                    cursor += 2;
                }
                48 if num_params - cursor >= 4 && self.params[cursor] == 2 => {
                    op.set_bg_color(Pixel {
                        red: self.params[cursor + 1] as u8,
                        green: self.params[cursor + 2] as u8,
                        blue: self.params[cursor + 3] as u8,
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
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                x: 0,
                y: 0,
                fg: Pixel::WHITE,
                bg: Pixel::BLACK,
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

        // There is invalid as there is no 0-th row or 0-th column. But in this case, let's move
        // the cursor to the first row and the first column.
        eat_escape_sequence(&mut esc_fsm, &mut state, b"\x1B[0;0H");
        assert_eq!(state.x, 0);
        assert_eq!(state.y, 0);

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
