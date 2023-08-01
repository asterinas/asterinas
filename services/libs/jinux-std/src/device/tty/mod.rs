use spin::Once;

use self::driver::TtyDriver;
use self::line_discipline::LineDiscipline;
use super::*;
use crate::fs::utils::{IoEvents, IoctlCmd, Poller};
use crate::prelude::*;
use crate::process::process_group::ProcessGroup;
use crate::process::process_table;
use crate::util::{read_val_from_user, write_val_to_user};

pub mod driver;
pub mod line_discipline;
pub mod termio;

static N_TTY: Once<Arc<Tty>> = Once::new();

pub(super) fn init() {
    let name = CString::new("console").unwrap();
    let tty = Arc::new(Tty::new(name));
    N_TTY.call_once(|| tty);
    driver::init();
}

pub struct Tty {
    /// tty_name
    name: CString,
    /// line discipline
    ldisc: LineDiscipline,
    /// driver
    driver: SpinLock<Weak<TtyDriver>>,
}

impl Tty {
    pub fn new(name: CString) -> Self {
        Tty {
            name,
            ldisc: LineDiscipline::new(),
            driver: SpinLock::new(Weak::new()),
        }
    }

    /// Set foreground process group
    pub fn set_fg(&self, process_group: Weak<ProcessGroup>) {
        self.ldisc.set_fg(process_group);
    }

    pub fn set_driver(&self, driver: Weak<TtyDriver>) {
        *self.driver.lock_irq_disabled() = driver;
    }

    pub fn receive_char(&self, item: u8) {
        self.ldisc.push_char(item, |content| print!("{}", content));
    }
}

impl Device for Tty {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(5, 0)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.ldisc.read(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if let Ok(content) = alloc::str::from_utf8(buf) {
            print!("{content}");
        } else {
            println!("Not utf-8 content: {:?}", buf);
        }
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.ldisc.poll(mask, poller)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                // Get terminal attributes
                let termios = self.ldisc.termios();
                trace!("get termios = {:?}", termios);
                write_val_to_user(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                let Some(fg_pgid) = self.ldisc.fg_pgid() else {
                    return_errno_with_message!(Errno::ENOENT, "No fg process group")
                };
                debug!("fg_pgid = {}", fg_pgid);
                write_val_to_user(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
                // Set the process group id of fg progress group
                let pgid = read_val_from_user::<i32>(arg)?;
                match process_table::pgid_to_process_group(pgid) {
                    None => self.ldisc.set_fg(Weak::new()),
                    Some(process_group) => self.ldisc.set_fg(Arc::downgrade(&process_group)),
                }
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                // Set terminal attributes
                let termios = read_val_from_user(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                Ok(0)
            }
            IoctlCmd::TCSETSW => {
                let termios = read_val_from_user(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                // TODO: drain output buffer
                Ok(0)
            }
            IoctlCmd::TCSETSF => {
                let termios = read_val_from_user(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
                self.ldisc.drain_input();
                // TODO: drain output buffer
                Ok(0)
            }
            IoctlCmd::TIOCGWINSZ => {
                // TODO:get window size
                Ok(0)
            }
            _ => todo!(),
        }
    }
}

/// FIXME: should we maintain a static console?
pub fn get_n_tty() -> &'static Arc<Tty> {
    N_TTY.get().unwrap()
}
