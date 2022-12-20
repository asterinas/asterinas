
use self::line_discipline::LineDiscipline;
use crate::driver::console::receive_console_char;
use crate::fs::events::IoEvents;
use crate::fs::ioctl::IoctlCmd;
use crate::process::{process_table, Pgid};
use crate::util::{read_val_from_user, write_val_to_user};
use crate::{fs::file::File, prelude::*};

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
    ldisc: Mutex<LineDiscipline>,
}

impl Tty {
    pub fn new(name: CString) -> Self {
        Tty {
            name,
            ldisc: Mutex::new(LineDiscipline::new()),
        }
    }

    /// Set foreground process group
    pub fn set_fg(&self, pgid: Pgid) {
        self.ldisc.lock().set_fg(pgid);
    }

    /// Wake up foreground process group that wait on IO events.
    /// This function should be called when the interrupt handler of IO events is called.
    pub fn wake_fg_proc_grp(&self) {
        let ldisc = self.ldisc.lock();
        if let Some(fg_pgid) = ldisc.get_fg() {
            if let Some(fg_proc_grp) = process_table::pgid_to_process_group(*fg_pgid) {
                fg_proc_grp.wake_all_polling_procs();
            }
        }
    }
}

impl File for Tty {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.ldisc.lock().read(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if let Ok(content) = alloc::str::from_utf8(buf) {
            print!("{content}");
        } else {
            println!("Not utf-8 content: {:?}", buf);
        }
        Ok(buf.len())
    }

    fn poll(&self) -> IoEvents {
        if !self.ldisc.lock().is_empty() {
            return IoEvents::POLLIN;
        }
        // receive keyboard input
        let byte = receive_console_char();
        self.ldisc.lock().push_char(byte);
        return IoEvents::POLLIN;
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                // Get terminal attributes
                let ldist_lock = self.ldisc.lock();
                let termios = ldist_lock.get_termios();
                debug!("get termios = {:?}", termios);
                write_val_to_user(arg, termios)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                // FIXME: Get the process group ID of the foreground process group on this terminal.
                let ldist_lock = self.ldisc.lock();
                let fg_pgid = ldist_lock.get_fg();
                match fg_pgid {
                    None => return_errno_with_message!(Errno::ENOENT, "No fg process group"),
                    Some(fg_pgid) => {
                        debug!("fg_pgid = {}", fg_pgid);
                        write_val_to_user(arg, fg_pgid)?;
                        Ok(0)
                    }
                }
            }
            IoctlCmd::TIOCSPGRP => {
                // Set the process group id of fg progress group
                let pgid = read_val_from_user::<i32>(arg)?;
                let mut ldist_lock = self.ldisc.lock();
                ldist_lock.set_fg(pgid);
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                // Set terminal attributes
                let termios = read_val_from_user(arg)?;
                debug!("set termios = {:?}", termios);
                let mut ldist_lock = self.ldisc.lock();
                ldist_lock.set_termios(termios);
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
pub fn get_console() -> &'static Arc<Tty> {
    &N_TTY
}
