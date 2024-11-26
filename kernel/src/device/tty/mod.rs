// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use ostd::early_print;
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
        signal::{signals::kernel::KernelSignal, PollHandle, Pollable},
        JobControl, Process, Terminal,
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

pub struct Tty {
    /// tty_name
    name: CString,
    /// line discipline
    ldisc: Arc<LineDiscipline>,
    job_control: Arc<JobControl>,
    /// driver
    driver: SpinLock<Weak<TtyDriver>>,
    weak_self: Weak<Self>,
}

impl Tty {
    pub fn new(name: CString) -> Arc<Self> {
        let (job_control, ldisc) = new_job_control_and_ldisc();
        Arc::new_cyclic(move |weak_ref| Tty {
            name,
            ldisc,
            job_control,
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
        self.ldisc
            .push_char(ch, |content| early_print!("{}", content))
    }
}

impl Pollable for Tty {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.ldisc.poll(mask, poller)
    }
}

impl FileIo for Tty {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut buf = vec![0; writer.avail()];
        self.job_control.wait_until_in_foreground()?;
        let read_len = self.ldisc.read(buf.as_mut_slice())?;
        writer.write_fallible(&mut buf.as_slice().into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buf = reader.collect()?;
        if let Ok(content) = alloc::str::from_utf8(&buf) {
            print!("{content}");
        } else {
            println!("Not utf-8 content: {:?}", buf);
        }
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                // Get terminal attributes
                let termios = self.ldisc.termios();
                trace!("get termios = {:?}", termios);
                current_userspace!().write_val(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                let Some(foreground) = self.foreground() else {
                    return_errno_with_message!(Errno::ESRCH, "No fg process group")
                };
                let fg_pgid = foreground.pgid();
                debug!("fg_pgid = {}", fg_pgid);
                current_userspace!().write_val(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
                // Set the process group id of fg progress group
                let pgid = {
                    let pgid: i32 = current_userspace!().read_val(arg)?;
                    if pgid < 0 {
                        return_errno_with_message!(Errno::EINVAL, "negative pgid");
                    }
                    pgid as u32
                };

                self.set_foreground(&pgid)?;
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                // Set terminal attributes
                let termios = current_userspace!().read_val(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                Ok(0)
            }
            IoctlCmd::TCSETSW => {
                let termios = current_userspace!().read_val(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                // TODO: drain output buffer
                Ok(0)
            }
            IoctlCmd::TCSETSF => {
                let termios = current_userspace!().read_val(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                self.ldisc.drain_input();
                // TODO: drain output buffer
                Ok(0)
            }
            IoctlCmd::TIOCGWINSZ => {
                let winsize = self.ldisc.window_size();
                current_userspace!().write_val(arg, &winsize)?;
                Ok(0)
            }
            IoctlCmd::TIOCSWINSZ => {
                let winsize = current_userspace!().read_val(arg)?;
                self.ldisc.set_window_size(winsize);
                Ok(0)
            }
            IoctlCmd::TIOCSCTTY => {
                self.set_current_session()?;
                Ok(0)
            }
            _ => todo!(),
        }
    }
}

impl Terminal for Tty {
    fn arc_self(&self) -> Arc<dyn Terminal> {
        self.weak_self.upgrade().unwrap() as _
    }

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

pub fn new_job_control_and_ldisc() -> (Arc<JobControl>, Arc<LineDiscipline>) {
    let job_control = Arc::new(JobControl::new());

    let send_signal = {
        let cloned_job_control = job_control.clone();
        move |signal: KernelSignal| {
            let Some(foreground) = cloned_job_control.foreground() else {
                return;
            };

            foreground.broadcast_signal(signal);
        }
    };

    let ldisc = LineDiscipline::new(Arc::new(send_signal));

    (job_control, ldisc)
}

pub fn get_n_tty() -> &'static Arc<Tty> {
    N_TTY.get().unwrap()
}

/// Open `N_TTY` as the controlling terminal for the process. This method should
/// only be called when creating the init process.
pub fn open_ntty_as_controlling_terminal(process: &Process) -> Result<()> {
    let tty = get_n_tty();

    let session = &process.session().unwrap();
    let process_group = process.process_group().unwrap();

    session.set_terminal(|| {
        tty.job_control.set_session(session);
        Ok(tty.clone())
    })?;

    tty.job_control.set_foreground(Some(&process_group))?;

    Ok(())
}
