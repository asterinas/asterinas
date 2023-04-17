use crate::prelude::*;

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum IoctlCmd {
    // Get terminal attributes
    TCGETS = 0x5401,
    TCSETS = 0x5402,
    // Get the process group ID of the foreground process group on this terminal
    TIOCGPGRP = 0x540f,
    // Set the foreground process group ID of this terminal.
    TIOCSPGRP = 0x5410,
    // Set window size
    TIOCGWINSZ = 0x5413,
    TIOCSWINSZ = 0x5414,
}
