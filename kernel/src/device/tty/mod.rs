// SPDX-License-Identifier: MPL-2.0

use aster_console::BitmapFont;
use ostd::sync::LocalIrqDisabled;
use termio::CFontOp;

use self::line_discipline::LineDiscipline;
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
        utils::IoctlCmd,
    },
    prelude::*,
    process::{
        broadcast_signal_async,
        signal::{PollHandle, Pollable, Pollee},
        JobControl, Terminal,
    },
};

mod device;
mod driver;
mod line_discipline;
mod n_tty;
mod termio;

pub use device::TtyDevice;
pub use driver::{PushCharError, TtyDriver};
pub(super) use n_tty::init;
pub use n_tty::{iter_n_tty, system_console};

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
    weak_self: Weak<Self>,
}

impl<D> Tty<D> {
    pub fn new(index: u32, driver: D) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Tty {
            index,
            driver,
            ldisc: SpinLock::new(LineDiscipline::new()),
            job_control: JobControl::new(),
            pollee: Pollee::new(),
            weak_self: weak_ref.clone(),
        })
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn driver(&self) -> &D {
        &self.driver
    }

    /// Returns whether new characters can be pushed into the input buffer.
    ///
    /// This method should return `false` if the input buffer is full.
    pub fn can_push(&self) -> bool {
        !self.ldisc.lock().is_full()
    }

    /// Notifies that the output buffer now has room for new characters.
    ///
    /// This method should be called when the state of [`TtyDriver::can_push`] changes from `false`
    /// to `true`.
    pub fn notify_output(&self) {
        self.pollee.notify(IoEvents::OUT);
    }
}

impl<D: TtyDriver> Tty<D> {
    /// Pushes characters into the output buffer.
    ///
    /// This method returns the number of bytes pushed or fails with an error if no bytes can be
    /// pushed because the buffer is full.
    pub fn push_input(&self, chs: &[u8]) -> core::result::Result<usize, PushCharError> {
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
                return Err(PushCharError);
            } else if res.is_err() {
                break;
            } else {
                len += 1;
            }
        }

        self.pollee.notify(IoEvents::IN);
        Ok(len)
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.ldisc.lock().buffer_len() > 0 {
            events |= IoEvents::IN;
        }

        if self.driver.can_push() {
            events |= IoEvents::OUT;
        }

        events
    }

    fn handle_set_font(&self, font_op: &CFontOp) -> Result<()> {
        let CFontOp {
            op,
            flags: _,
            width,
            height,
            charcount,
            data,
        } = font_op;

        let vpitch = match *op {
            CFontOp::OP_SET => CFontOp::NONTALL_VPITCH,
            CFontOp::OP_SET_TALL => font_op.height,
            CFontOp::OP_SET_DEFAULT => return self.driver.set_font(BitmapFont::new_basic8x8()),
            _ => return_errno_with_message!(Errno::EINVAL, "the font operation is invalid"),
        };

        if *width == 0
            || *height == 0
            || *width > CFontOp::MAX_WIDTH
            || *height > CFontOp::MAX_HEIGHT
            || *charcount > CFontOp::MAX_CHARCOUNT
            || *height > vpitch
        {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid or too large");
        }

        let font_size = width.div_ceil(u8::BITS) * vpitch * charcount;
        let mut font_data = vec![0; font_size as usize];
        current_userspace!().read_bytes(*data as Vaddr, &mut (&mut font_data[..]).into())?;

        // In Linux, the most significant bit represents the first pixel, but `BitmapFont` requires
        // the least significant bit to represent the first pixel. So now we reverse the bits.
        font_data
            .iter_mut()
            .for_each(|byte| *byte = byte.reverse_bits());

        let font = BitmapFont::new_with_vpitch(
            *width as usize,
            *height as usize,
            vpitch as usize,
            font_data,
        );
        self.driver.set_font(font)?;

        Ok(())
    }
}

impl<D: TtyDriver> Pollable for Tty<D> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl<D: TtyDriver> FileIo for Tty<D> {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.job_control.wait_until_in_foreground()?;

        // TODO: Add support for non-blocking mode and timeout
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let read_len =
            self.wait_events(IoEvents::IN, None, || self.ldisc.lock().try_read(&mut buf))?;
        self.pollee.invalidate();
        self.driver.notify_input();

        // TODO: Confirm what we should do if `write_fallible` fails in the middle.
        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        // TODO: Add support for non-blocking mode and timeout
        let len = self.wait_events(IoEvents::OUT, None, || {
            Ok(self.driver.push_output(&buf[..write_len])?)
        })?;
        self.pollee.invalidate();
        Ok(len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                let termios = *self.ldisc.lock().termios();

                current_userspace!().write_val(arg, &termios)?;
            }
            IoctlCmd::TCSETS => {
                let termios = current_userspace!().read_val(arg)?;

                self.ldisc.lock().set_termios(termios);
            }
            IoctlCmd::TCSETSW => {
                let termios = current_userspace!().read_val(arg)?;

                let mut ldisc = self.ldisc.lock();
                ldisc.set_termios(termios);
                self.driver.drain_output();
            }
            IoctlCmd::TCSETSF => {
                let termios = current_userspace!().read_val(arg)?;

                let mut ldisc = self.ldisc.lock();
                ldisc.set_termios(termios);
                ldisc.drain_input();
                self.driver.drain_output();

                self.pollee.invalidate();
            }
            IoctlCmd::TIOCGWINSZ => {
                let winsize = self.ldisc.lock().window_size();

                current_userspace!().write_val(arg, &winsize)?;
            }
            IoctlCmd::TIOCSWINSZ => {
                let winsize = current_userspace!().read_val(arg)?;

                self.ldisc.lock().set_window_size(winsize);
            }
            IoctlCmd::TIOCGPTN => {
                let idx = self.index;

                current_userspace!().write_val(arg, &idx)?;
            }
            IoctlCmd::FIONREAD => {
                let buffer_len = self.ldisc.lock().buffer_len() as u32;

                current_userspace!().write_val(arg, &buffer_len)?;
            }
            IoctlCmd::KDFONTOP => {
                let font_op = current_userspace!().read_val(arg)?;

                self.handle_set_font(&font_op)?;
            }
            _ => (self.weak_self.upgrade().unwrap() as Arc<dyn Terminal>)
                .job_ioctl(cmd, arg, false)?,
        }

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
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(88, self.index)
    }
}
