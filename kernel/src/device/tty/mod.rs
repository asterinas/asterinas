// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};
use ostd::sync::LocalIrqDisabled;

use self::{line_discipline::LineDiscipline, termio::CFontOp};
use crate::{
    device::{Device, DeviceType},
    events::IoEvents,
    fs::file::{FileIo, StatusFlags},
    prelude::*,
    process::{
        JobControl, Terminal, broadcast_signal_async,
        signal::{PollHandle, Pollable, Pollee},
    },
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

mod device;
mod driver;
mod file;
mod flags;
mod hvc;
pub(super) mod ioctl_defs;
mod line_discipline;
mod serial;
pub(super) mod termio;
mod vt;

pub use device::SystemConsole;
pub use driver::TtyDriver;
pub(super) use flags::TtyFlags;

pub(super) fn init_in_first_process() -> Result<()> {
    hvc::init_in_first_process()?;
    serial::init_in_first_process()?;
    device::init_in_first_process()?;
    vt::init_in_first_process()?;

    Ok(())
}

const IO_CAPACITY: usize = 4096;

/// A teletyper (TTY).
///
/// This abstracts the general functionality of a TTY in a way that
///  - Any input device driver can use [`Tty::push_input`] to push input characters, and users can
///    [`Tty::read`] from the TTY;
///  - Users can also [`Tty::write`] output characters to the TTY and the output device driver will
///    receive the characters from [`TtyDriver::push_output`] where the generic parameter `D` is
///    the [`TtyDriver`].
///
/// ```text
/// +------------+     +-------------+
/// |input device|     |output device|
/// |   driver   |     |   driver    |
/// +-----+------+     +------^------+
///       |                   |
///       |     +-------+     |
///       +----->  TTY  +-----+
///             +-------+
/// Tty::push_input   D::push_output
/// ```
pub struct Tty<D> {
    index: u32,
    driver: D,
    ldisc: SpinLock<LineDiscipline, LocalIrqDisabled>,
    job_control: JobControl,
    pollee: Pollee,
    tty_flags: TtyFlags,
    weak_self: Weak<Self>,
}

impl<D> Tty<D> {
    pub(super) fn new(index: u32, driver: D) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Tty {
            index,
            driver,
            ldisc: SpinLock::new(LineDiscipline::new()),
            job_control: JobControl::new(),
            pollee: Pollee::new(),
            tty_flags: TtyFlags::new(),
            weak_self: weak_ref.clone(),
        })
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub(super) fn driver(&self) -> &D {
        &self.driver
    }

    /// Returns whether new characters can be pushed into the input buffer.
    ///
    /// This method should return `false` if the input buffer is full.
    pub(super) fn can_push(&self) -> bool {
        !self.ldisc.lock().is_full()
    }

    /// Notifies that the output buffer now has room for new characters.
    ///
    /// This method should be called when the state of [`TtyDriver::can_push`] changes from `false`
    /// to `true`.
    pub(super) fn notify_output(&self) {
        self.pollee.notify(IoEvents::OUT);
    }

    /// Notifies that the other end has been closed.
    pub(super) fn notify_hup(&self) {
        self.pollee.notify(IoEvents::ERR | IoEvents::HUP);
    }

    /// Returns the TTY flags.
    pub(super) fn tty_flags(&self) -> &TtyFlags {
        &self.tty_flags
    }
}

impl<D: TtyDriver> Tty<D> {
    /// Pushes characters into the output buffer.
    ///
    /// This method returns the number of bytes pushed or fails with an error if no bytes can be
    /// pushed because the buffer is full.
    pub fn push_input(&self, chs: &[u8]) -> Result<usize> {
        let mut ldisc = self.ldisc.lock();
        let mut echo = self.driver.echo_callback();

        let mut len = 0;
        for ch in chs {
            let res = ldisc.push_char(
                *ch,
                |signum| {
                    if let Some(foreground) = self.job_control.foreground() {
                        broadcast_signal_async(Arc::downgrade(&foreground), signum);
                    }
                },
                &mut echo,
            );
            if res.is_err() && len == 0 {
                return_errno_with_message!(Errno::EAGAIN, "the line discipline is full");
            } else if res.is_err() {
                break;
            } else {
                len += 1;
            }
        }

        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        Ok(len)
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.ldisc.lock().buffer_len() > 0 {
            events |= IoEvents::IN | IoEvents::RDNORM;
        }

        if self.driver.can_push() {
            events |= IoEvents::OUT;
        }

        if self.tty_flags.is_other_closed() {
            events |= IoEvents::ERR | IoEvents::HUP;
        }

        events
    }
}

impl<D: TtyDriver> Pollable for Tty<D> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl<D: TtyDriver> Tty<D> {
    pub fn read(&self, writer: &mut VmWriter, status_flags: StatusFlags) -> Result<usize> {
        if self.tty_flags.is_other_closed() {
            return Ok(0);
        }

        self.job_control.wait_until_in_foreground()?;

        // TODO: Add support for timeout.
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);
        let read_len = if is_nonblocking {
            self.ldisc.lock().try_read(&mut buf)?
        } else {
            self.wait_events(IoEvents::IN, None, || self.ldisc.lock().try_read(&mut buf))?
        };
        self.pollee.invalidate();
        self.driver.notify_input();

        // TODO: Confirm what we should do if `write_fallible` fails in the middle.
        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    pub fn write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        if self.tty_flags.is_other_closed() {
            return_errno_with_message!(Errno::EIO, "the TTY is closed");
        }

        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        // TODO: Add support for timeout.
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);
        let len = if is_nonblocking {
            self.driver.push_output(&buf[..write_len])?
        } else {
            self.wait_events(IoEvents::OUT, None, || {
                self.driver.push_output(&buf[..write_len])
            })?
        };
        self.pollee.invalidate();
        Ok(len)
    }

    pub fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        use crate::util::ioctl::common_defs::GetNumBytesToRead;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetTermios => {
                let termios = *self.ldisc.lock().termios();

                cmd.write(&termios)?;
            }
            cmd @ SetTermios => {
                let termios = cmd.read()?;

                let mut ldisc = self.ldisc.lock();
                let old_termios = ldisc.termios();
                self.driver().on_termios_change(old_termios, &termios);
                ldisc.set_termios(termios);
            }
            cmd @ SetTermiosWait => {
                let termios = cmd.read()?;

                // TODO: If applicable, wait for the output buffer to drain. For now, we don't need
                // to do anything here because:
                //  - Linux does not consider a pty to have an output buffer, so it does not drain
                //    it. See
                //    <https://elixir.bootlin.com/linux/v5.10.247/source/drivers/tty/pty.c#L137-L148>.
                //  - We don't currently have an output buffer for other TTYs.
                let mut ldisc = self.ldisc.lock();
                let old_termios = ldisc.termios();
                self.driver().on_termios_change(old_termios, &termios);
                ldisc.set_termios(termios);
            }
            cmd @ SetTermiosFlush => {
                let termios = cmd.read()?;

                // TODO: If applicable, wait for the output buffer to drain. (See comments above.)
                let mut ldisc = self.ldisc.lock();
                let old_termios = ldisc.termios();
                self.driver().on_termios_change(old_termios, &termios);
                ldisc.set_termios(termios);
                ldisc.drain_input();

                self.pollee.invalidate();
            }
            cmd @ GetWinSize => {
                let winsize = self.ldisc.lock().window_size();

                cmd.write(&winsize)?;
            }
            cmd @ SetWinSize => {
                let winsize = cmd.read()?;

                self.ldisc.lock().set_window_size(winsize);
            }
            cmd @ GetNumBytesToRead => {
                if self.tty_flags.is_other_closed() {
                    return_errno_with_message!(Errno::EIO, "the TTY is closed");
                }

                let buffer_len = self.ldisc.lock().buffer_len() as i32;

                cmd.write(&buffer_len)?;
            }

            _ => {
                let terminal = self.weak_self.upgrade().unwrap() as Arc<dyn Terminal>;

                // Process job-control ioctls.
                if terminal.job_ioctl(raw_ioctl, false)? {
                    return Ok(0);
                }

                // Process driver-specific ioctls.
                if self.driver.ioctl(self, raw_ioctl)? {
                    return Ok(0);
                }

                return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
            }
        });

        Ok(0)
    }
}

impl<D: TtyDriver> Terminal for Tty<D> {
    fn job_control(&self) -> &JobControl {
        &self.job_control
    }
}

impl<D: TtyDriver> Device for Tty<D> {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(
            MajorId::new(D::DEVICE_MAJOR_ID as u16),
            MinorId::new(self.index),
        )
    }

    fn devtmpfs_path(&self) -> Option<String> {
        self.driver.devtmpfs_path(self.index)
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        D::open(self.weak_self.upgrade().unwrap())
    }
}
