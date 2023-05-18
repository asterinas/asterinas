use self::line_discipline::LineDiscipline;
use super::*;
use crate::driver::tty::TtyDriver;
use crate::fs::utils::{IoEvents, IoctlCmd, Poller};
use crate::prelude::*;
use crate::process::Pgid;
use crate::util::{read_val_from_user, write_val_to_user};

pub mod line_discipline;
pub mod termio;

lazy_static! {
    static ref N_TTY: Arc<Tty> = {
        let name = CString::new("console").unwrap();
        Arc::new(Tty::new(name))
    };
}

pub struct Tty {
    /// tty_name
    name: CString,
    /// line discipline
    ldisc: LineDiscipline,
    /// driver
    driver: Mutex<Weak<TtyDriver>>,
}

impl Tty {
    pub fn new(name: CString) -> Self {
        Tty {
            name,
            ldisc: LineDiscipline::new(),
            driver: Mutex::new(Weak::new()),
        }
    }

    /// Set foreground process group
    pub fn set_fg(&self, pgid: Pgid) {
        self.ldisc.set_fg(pgid);
    }

    pub fn set_driver(&self, driver: Weak<TtyDriver>) {
        *self.driver.lock() = driver;
    }

    pub fn receive_char(&self, item: u8) {
        self.ldisc.push_char(item);
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
                let termios = self.ldisc.get_termios();
                trace!("get termios = {:?}", termios);
                write_val_to_user(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                // FIXME: Get the process group ID of the foreground process group on this terminal.
                let fg_pgid = self.ldisc.get_fg();
                match fg_pgid {
                    None => return_errno_with_message!(Errno::ENOENT, "No fg process group"),
                    Some(fg_pgid) => {
                        debug!("fg_pgid = {}", fg_pgid);
                        write_val_to_user(arg, &fg_pgid)?;
                        Ok(0)
                    }
                }
            }
            IoctlCmd::TIOCSPGRP => {
                // Set the process group id of fg progress group
                let pgid = read_val_from_user::<i32>(arg)?;
                self.ldisc.set_fg(pgid);
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                // Set terminal attributes
                let termios = read_val_from_user(arg)?;
                debug!("set termios = {:?}", termios);
                self.ldisc.set_termios(termios);
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
    &N_TTY
}
