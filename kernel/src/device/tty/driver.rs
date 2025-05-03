// SPDX-License-Identifier: MPL-2.0

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
    fn push_output(&self, chs: &[u8]);

    /// Drains the output buffer.
    fn drain_output(&self);

    /// Returns a callback function that echoes input characters to the output buffer.
    ///
    /// Note that the implementation may choose to hold a lock during the life of the callback.
    /// During this time, calls to other methods such as [`Self::push_output`] may cause deadlocks.
    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_;
}
