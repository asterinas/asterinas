// SPDX-License-Identifier: MPL-2.0

use ostd::{early_print, sync::LocalIrqDisabled};
use spin::Once;

use self::{driver::TtyDriver, line_discipline::LineDiscipline};
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
pub mod driver;
pub mod line_discipline;
pub mod termio;

pub use device::TtyDevice;

static N_TTY: Once<Arc<Tty>> = Once::new();

pub(super) fn init() {
    let name = CString::new("console").unwrap();
    let tty = Tty::new(name);
    N_TTY.call_once(|| tty);
    driver::init();
}

const IO_CAPACITY: usize = 4096;

pub struct Tty {
    /// tty_name
    #[expect(unused)]
    name: CString,
    /// line discipline
    ldisc: SpinLock<LineDiscipline, LocalIrqDisabled>,
    job_control: JobControl,
    pollee: Pollee,
    /// driver
    driver: SpinLock<Weak<TtyDriver>>,
    weak_self: Weak<Self>,
}

impl Tty {
    pub fn new(name: CString) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Tty {
            name,
            ldisc: SpinLock::new(LineDiscipline::new()),
            job_control: JobControl::new(),
            pollee: Pollee::new(),
            driver: SpinLock::new(Weak::new()),
            weak_self: weak_ref.clone(),
        })
    }

    pub fn set_driver(&self, driver: Weak<TtyDriver>) {
        *self.driver.disable_irq().lock() = driver;
    }

    pub fn push_char(&self, ch: u8) {
        // FIXME: Use `early_print` to avoid calling virtio-console.
        // This is only a workaround
        self.ldisc.lock().push_char(
            ch,
            |signum| {
                if let Some(foreground) = self.job_control.foreground() {
                    broadcast_signal_async(Arc::downgrade(&foreground), signum);
                }
            },
            |content| early_print!("{}", content),
        );
        self.pollee.notify(IoEvents::IN);
    }

    fn check_io_events(&self) -> IoEvents {
        if self.ldisc.lock().buffer_len() != 0 {
            IoEvents::IN | IoEvents::OUT
        } else {
            IoEvents::OUT
        }
    }
}

impl Pollable for Tty {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileIo for Tty {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.job_control.wait_until_in_foreground()?;

        // TODO: Add support for non-blocking mode and timeout
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let read_len =
            self.wait_events(IoEvents::IN, None, || self.ldisc.lock().try_read(&mut buf))?;
        self.pollee.invalidate();

        // TODO: Confirm what we should do if `write_fallible` fails in the middle.
        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        if let Ok(content) = alloc::str::from_utf8(&buf[..write_len]) {
            print!("{content}");
        } else {
            println!("Not utf-8 content: {:?}", buf);
        }
        Ok(write_len)
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

                self.ldisc.lock().set_termios(termios);
                // TODO: Drain the output buffer
            }
            IoctlCmd::TCSETSF => {
                let termios = current_userspace!().read_val(arg)?;

                let mut ldisc = self.ldisc.lock();
                ldisc.set_termios(termios);
                ldisc.drain_input();
                // TODO: Drain the output buffer

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
            _ => (self.weak_self.upgrade().unwrap() as Arc<dyn Terminal>)
                .job_ioctl(cmd, arg, false)?,
        }

        Ok(0)
    }
}

impl Terminal for Tty {
    fn job_control(&self) -> &JobControl {
        &self.job_control
    }
}

impl Device for Tty {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // The same value as /dev/console in linux.
        DeviceId::new(88, 0)
    }
}

pub fn get_n_tty() -> &'static Arc<Tty> {
    N_TTY.get().unwrap()
}
