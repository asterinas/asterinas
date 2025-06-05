// SPDX-License-Identifier: MPL-2.0

use crate::prelude::{Errno, Error, Result};

/// An error indicating that no characters can be pushed because the buffer is full.
#[derive(Debug, Clone, Copy)]
pub struct PushCharError;

impl From<PushCharError> for Error {
    fn from(_value: PushCharError) -> Self {
        Error::with_message(Errno::EAGAIN, "the buffer is full")
    }
}

/// A TTY driver.
///
/// A driver exposes some device-specific behavior to [`Tty`]. For example, a device provides
/// methods to write to the output buffer (see [`Self::push_output`]), where the output buffer can
/// be the monitor if the underlying device is framebuffer, or just a ring buffer if the underlying
/// device is pseduoterminal).
///
/// [`Tty`]: super::Tty
pub trait TtyDriver: Send + Sync + 'static {
    /// Pushes characters into the output buffer.
    ///
    /// This method returns the number of bytes pushed or fails with an error if no bytes can be
    /// pushed because the buffer is full.
    fn push_output(&self, chs: &[u8]) -> core::result::Result<usize, PushCharError>;

    /// Drains the output buffer.
    fn drain_output(&self);

    /// Returns a callback function that echoes input characters to the output buffer.
    ///
    /// Note that the implementation may choose to hold a lock during the life of the callback.
    /// During this time, calls to other methods such as [`Self::push_output`] may cause deadlocks.
    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_;

    /// Returns whether new characters can be pushed into the output buffer.
    ///
    /// This method should return `false` if the output buffer is full.
    fn can_push(&self) -> bool;

    /// Notifies that the input buffer now has room for new characters.
    ///
    /// This method should be called when the state of [`Tty::can_push`] changes from `false` to
    /// `true`.
    ///
    /// [`Tty::can_push`]: super::Tty::can_push
    fn notify_input(&self);

    /// Sets the TTY font.
    fn set_font(&self, font: aster_console::BitmapFont) -> Result<()>;
}
