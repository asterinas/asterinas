use alloc::format;
use ringbuf::{ring_buffer::RbBase, HeapRb, Rb};

use crate::device::tty::line_discipline::LineDiscipline;
use crate::device::tty::new_job_control_and_ldisc;
use crate::events::IoEvents;
use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::fs::devpts::DevPts;
use crate::fs::fs_resolver::FsPath;
use crate::fs::inode_handle::FileIo;
use crate::fs::utils::{AccessMode, Inode, InodeMode, IoctlCmd};
use crate::prelude::*;
use crate::process::signal::{Pollee, Poller};
use crate::process::{JobControl, Terminal};
use crate::util::{read_val_from_user, write_val_to_user};

const BUFFER_CAPACITY: usize = 4096;

/// Pesudo terminal master.
/// Internally, it has two buffers.
/// One is inside ldisc, which is written by master and read by slave,
/// the other is a ring buffer, which is written by slave and read by master.
pub struct PtyMaster {
    ptmx: Arc<dyn Inode>,
    index: u32,
    output: Arc<LineDiscipline>,
    input: SpinLock<HeapRb<u8>>,
    job_control: Arc<JobControl>,
    /// The state of input buffer
    pollee: Pollee,
    weak_self: Weak<Self>,
}

impl PtyMaster {
    pub fn new(ptmx: Arc<dyn Inode>, index: u32) -> Arc<Self> {
        let (job_control, ldisc) = new_job_control_and_ldisc();
        Arc::new_cyclic(move |weak_ref| PtyMaster {
            ptmx,
            index,
            output: ldisc,
            input: SpinLock::new(HeapRb::new(BUFFER_CAPACITY)),
            job_control,
            pollee: Pollee::new(IoEvents::OUT),
            weak_self: weak_ref.clone(),
        })
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn ptmx(&self) -> &Arc<dyn Inode> {
        &self.ptmx
    }

    pub(super) fn slave_push_char(&self, ch: u8) {
        let mut input = self.input.lock_irq_disabled();
        input.push_overwrite(ch);
        self.update_state(&input);
    }

    pub(super) fn slave_poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let mut poll_status = IoEvents::empty();

        let poll_in_mask = mask & IoEvents::IN;
        if !poll_in_mask.is_empty() {
            let poll_in_status = self.output.poll(poll_in_mask, poller);
            poll_status |= poll_in_status;
        }

        let poll_out_mask = mask & IoEvents::OUT;
        if !poll_out_mask.is_empty() {
            let poll_out_status = self.pollee.poll(poll_out_mask, poller);
            poll_status |= poll_out_status;
        }

        poll_status
    }

    pub(super) fn slave_buf_len(&self) -> usize {
        self.output.buffer_len()
    }

    fn update_state(&self, buf: &HeapRb<u8>) {
        if buf.is_empty() {
            self.pollee.del_events(IoEvents::IN)
        } else {
            self.pollee.add_events(IoEvents::IN);
        }
    }
}

impl FileIo for PtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // TODO: deal with nonblocking read
        if buf.is_empty() {
            return Ok(0);
        }

        let poller = Poller::new();
        loop {
            let mut input = self.input.lock_irq_disabled();

            if input.is_empty() {
                let events = self.pollee.poll(IoEvents::IN, Some(&poller));

                if events.contains(IoEvents::ERR) {
                    return_errno_with_message!(Errno::EACCES, "unexpected err");
                }

                if events.is_empty() {
                    drop(input);
                    // FIXME: deal with pty read timeout
                    poller.wait()?;
                }
                continue;
            }

            let read_len = input.len().min(buf.len());
            input.pop_slice(&mut buf[..read_len]);
            self.update_state(&input);
            return Ok(read_len);
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut input = self.input.lock();
        for character in buf {
            self.output.push_char(*character, |content| {
                for byte in content.as_bytes() {
                    input.push_overwrite(*byte);
                }
            });
        }

        self.update_state(&input);
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                let termios = self.output.termios();
                write_val_to_user(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                let termios = read_val_from_user(arg)?;
                self.output.set_termios(termios);
                Ok(0)
            }
            IoctlCmd::TIOCSPTLCK => {
                // TODO: lock/unlock pty
                Ok(0)
            }
            IoctlCmd::TIOCGPTN => {
                let idx = self.index();
                write_val_to_user(arg, &idx)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPTPEER => {
                let current = current!();

                // TODO: deal with open options
                let slave = {
                    let slave_name = {
                        let devpts_path = super::DEV_PTS.get().unwrap().abs_path();
                        format!("{}/{}", devpts_path, self.index())
                    };

                    let fs_path = FsPath::try_from(slave_name.as_str())?;

                    let inode_handle = {
                        let fs = current.fs().read();
                        let flags = AccessMode::O_RDWR as u32;
                        let mode = (InodeMode::S_IRUSR | InodeMode::S_IWUSR).bits();
                        fs.open(&fs_path, flags, mode)?
                    };
                    Arc::new(inode_handle)
                };

                let fd = {
                    let mut file_table = current.file_table().lock();
                    file_table.insert(slave)
                };
                Ok(fd)
            }
            IoctlCmd::TIOCGWINSZ => {
                let winsize = self.output.window_size();
                write_val_to_user(arg, &winsize)?;
                Ok(0)
            }
            IoctlCmd::TIOCSWINSZ => {
                let winsize = read_val_from_user(arg)?;
                self.output.set_window_size(winsize);
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                let Some(foreground) = self.foreground() else {
                    return_errno_with_message!(
                        Errno::ESRCH,
                        "the foreground process group does not exist"
                    );
                };
                let fg_pgid = foreground.pgid();
                write_val_to_user(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
                let pgid = {
                    let pgid: i32 = read_val_from_user(arg)?;
                    if pgid < 0 {
                        return_errno_with_message!(Errno::EINVAL, "negative pgid");
                    }
                    pgid as u32
                };

                self.set_foreground(&pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSCTTY => {
                self.set_current_session()?;
                Ok(0)
            }
            IoctlCmd::TIOCNOTTY => {
                self.release_current_session()?;
                Ok(0)
            }
            IoctlCmd::FIONREAD => {
                let len = self.input.lock().len() as i32;
                write_val_to_user(arg, &len)?;
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let mut poll_status = IoEvents::empty();

        let poll_in_mask = mask & IoEvents::IN;
        if !poll_in_mask.is_empty() {
            let poll_in_status = self.pollee.poll(poll_in_mask, poller);
            poll_status |= poll_in_status;
        }

        let poll_out_mask = mask & IoEvents::OUT;
        if !poll_out_mask.is_empty() {
            let poll_out_status = self.output.poll(poll_out_mask, poller);
            poll_status |= poll_out_status;
        }

        poll_status
    }
}

impl Terminal for PtyMaster {
    fn arc_self(&self) -> Arc<dyn Terminal> {
        self.weak_self.upgrade().unwrap() as _
    }

    fn job_control(&self) -> &JobControl {
        &self.job_control
    }
}

impl Drop for PtyMaster {
    fn drop(&mut self) {
        let fs = self.ptmx.fs();
        let devpts = fs.downcast_ref::<DevPts>().unwrap();

        let index = self.index;
        devpts.remove_slave(index);
    }
}

pub struct PtySlave {
    master: Weak<PtyMaster>,
    job_control: JobControl,
    weak_self: Weak<Self>,
}

impl PtySlave {
    pub fn new(master: &Arc<PtyMaster>) -> Arc<Self> {
        Arc::new_cyclic(|weak_ref| PtySlave {
            master: Arc::downgrade(master),
            job_control: JobControl::new(),
            weak_self: weak_ref.clone(),
        })
    }

    pub fn index(&self) -> u32 {
        self.master().index()
    }

    fn master(&self) -> Arc<PtyMaster> {
        self.master.upgrade().unwrap()
    }
}

impl Device for PtySlave {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> crate::fs::device::DeviceId {
        DeviceId::new(88, self.index())
    }
}

impl Terminal for PtySlave {
    fn arc_self(&self) -> Arc<dyn Terminal> {
        self.weak_self.upgrade().unwrap() as _
    }

    fn job_control(&self) -> &JobControl {
        &self.job_control
    }
}

impl FileIo for PtySlave {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.job_control.wait_until_in_foreground()?;
        self.master().output.read(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let master = self.master();
        for ch in buf {
            // do we need to add '\r' here?
            if *ch == b'\n' {
                master.slave_push_char(b'\r');
                master.slave_push_char(b'\n');
            } else {
                master.slave_push_char(*ch);
            }
        }
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.master().slave_poll(mask, poller)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS
            | IoctlCmd::TCSETS
            | IoctlCmd::TIOCGPTN
            | IoctlCmd::TIOCGWINSZ
            | IoctlCmd::TIOCSWINSZ => self.master().ioctl(cmd, arg),
            IoctlCmd::TIOCGPGRP => {
                if !self.is_controlling_terminal() {
                    return_errno_with_message!(Errno::ENOTTY, "slave is not controlling terminal");
                }

                let Some(foreground) = self.foreground() else {
                    return_errno_with_message!(
                        Errno::ESRCH,
                        "the foreground process group does not exist"
                    );
                };

                let fg_pgid = foreground.pgid();
                write_val_to_user(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
                let pgid = {
                    let pgid: i32 = read_val_from_user(arg)?;
                    if pgid < 0 {
                        return_errno_with_message!(Errno::EINVAL, "negative pgid");
                    }
                    pgid as u32
                };

                self.set_foreground(&pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSCTTY => {
                self.set_current_session()?;
                Ok(0)
            }
            IoctlCmd::TIOCNOTTY => {
                self.release_current_session()?;
                Ok(0)
            }
            IoctlCmd::FIONREAD => {
                let buffer_len = self.master().slave_buf_len() as i32;
                write_val_to_user(arg, &buffer_len)?;
                Ok(0)
            }
            _ => Ok(0),
        }
    }
}
