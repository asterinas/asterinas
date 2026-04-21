// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::{
        DevtmpfsInodeMeta,
        tty::{Tty, termio::CTermios},
    },
    fs::file::FileIo,
    prelude::*,
    util::ioctl::RawIoctl,
};

/// A TTY driver.
///
/// A driver exposes some device-specific behavior to [`Tty`]. For example, a device provides
/// methods to write to the output buffer (see [`Self::push_output`]), where the output buffer can
/// be the monitor if the underlying device is framebuffer, or just a ring buffer if the underlying
/// device is pseduoterminal).
///
/// [`Tty`]: super::Tty
pub trait TtyDriver: Send + Sync + 'static {
    /// The device major ID.
    const DEVICE_MAJOR_ID: u32;

    /// Returns the metadata that specifies a TTY device inode to be created in devtmpfs, if any.
    fn devtmpfs_meta(&self, index: u32) -> Option<DevtmpfsInodeMeta<'_>>;

    /// Opens the TTY.
    ///
    /// This function will be called when opening `/dev/tty`.
    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>>
    where
        Self: Sized;

    /// Pushes characters into the output buffer.
    ///
    /// This method returns the number of bytes pushed or fails with an error if no bytes can be
    /// pushed.
    fn push_output(&self, chs: &[u8]) -> Result<usize>;

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

    /// Notifies that the TTY termios is changed.
    ///
    /// This method will be called with a spin lock held, so it cannot break atomic mode.
    fn on_termios_change(&self, old_termios: &CTermios, new_termios: &CTermios);

    /// Handles driver-specific ioctl.
    ///
    /// This method allows a TTY driver to handle driver-specific
    /// ioctl commands that are not processed by the generic TTY layer.
    ///
    /// The return value depends on whether the ioctl is recognized:
    /// - If the driver recognizes and handles the ioctl, it should return
    ///   `Ok(true)`.
    /// - If an error occurs while processing the ioctl, it should return
    ///   `Err(_)`.
    /// - If the driver does not recognize the ioctl command, it should return
    ///   `Ok(false)` to indicate that the ioctl command is not supported.
    fn ioctl(&self, _tty: &Tty<Self>, _raw_ioctl: RawIoctl) -> Result<bool>
    where
        Self: Sized,
    {
        Ok(false)
    }
}
