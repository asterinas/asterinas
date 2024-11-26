// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    current_userspace,
    device::tty::{line_discipline::LineDiscipline, new_job_control_and_ldisc},
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        devpts::DevPts,
        file_table::FdFlags,
        fs_resolver::FsPath,
        inode_handle::FileIo,
        utils::{AccessMode, Inode, InodeMode, IoctlCmd},
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable, Pollee},
        JobControl, Terminal,
    },
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 4096;

/// Pesudo terminal master.
/// Internally, it has two buffers.
/// One is inside ldisc, which is written by master and read by slave,
/// the other is a ring buffer, which is written by slave and read by master.
pub struct PtyMaster {
    ptmx: Arc<dyn Inode>,
    index: u32,
    output: Arc<LineDiscipline>,
    input: SpinLock<RingBuffer<u8>>,
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
            input: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            job_control,
            pollee: Pollee::new(),
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
        let mut input = self.input.disable_irq().lock();
        input.push_overwrite(ch);
        self.pollee.notify(IoEvents::IN);
    }

    pub(super) fn slave_poll(
        &self,
        mask: IoEvents,
        mut poller: Option<&mut PollHandle>,
    ) -> IoEvents {
        let mut poll_status = IoEvents::empty();

        let poll_in_mask = mask & IoEvents::IN;
        if !poll_in_mask.is_empty() {
            let poll_in_status = self.output.poll(poll_in_mask, poller.as_deref_mut());
            poll_status |= poll_in_status;
        }

        let poll_out_mask = mask & IoEvents::OUT;
        if !poll_out_mask.is_empty() {
            let poll_out_status = self
                .pollee
                .poll_with(poll_out_mask, poller, || self.check_io_events());
            poll_status |= poll_out_status;
        }

        poll_status
    }

    pub(super) fn slave_buf_len(&self) -> usize {
        self.output.buffer_len()
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut input = self.input.disable_irq().lock();

        if input.is_empty() {
            return_errno_with_message!(Errno::EAGAIN, "the buffer is empty");
        }

        let read_len = input.read_fallible(writer)?;
        self.pollee.invalidate();

        Ok(read_len)
    }

    fn check_io_events(&self) -> IoEvents {
        let input = self.input.disable_irq().lock();

        if !input.is_empty() {
            IoEvents::IN | IoEvents::OUT
        } else {
            IoEvents::OUT
        }
    }
}

impl Pollable for PtyMaster {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        let mut poll_status = IoEvents::empty();

        let poll_in_mask = mask & IoEvents::IN;
        if !poll_in_mask.is_empty() {
            let poll_in_status = self
                .pollee
                .poll_with(poll_in_mask, poller.as_deref_mut(), || {
                    self.check_io_events()
                });
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

impl FileIo for PtyMaster {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !writer.has_avail() {
            return Ok(0);
        }

        // TODO: deal with nonblocking and timeout
        self.wait_events(IoEvents::IN, None, || self.try_read(writer))
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buf = reader.collect()?;
        let write_len = buf.len();
        let mut input = self.input.lock();
        for character in buf {
            self.output.push_char(character, |content| {
                for byte in content.as_bytes() {
                    input.push_overwrite(*byte);
                }
            });
        }

        self.pollee.notify(IoEvents::IN);
        Ok(write_len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                let termios = self.output.termios();
                current_userspace!().write_val(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                let termios = current_userspace!().read_val(arg)?;
                self.output.set_termios(termios);
                Ok(0)
            }
            IoctlCmd::TIOCSPTLCK => {
                // TODO: lock/unlock pty
                Ok(0)
            }
            IoctlCmd::TIOCGPTN => {
                let idx = self.index();
                current_userspace!().write_val(arg, &idx)?;
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
                    // TODO: deal with the O_CLOEXEC flag
                    file_table.insert(slave, FdFlags::empty())
                };
                Ok(fd)
            }
            IoctlCmd::TIOCGWINSZ => {
                let winsize = self.output.window_size();
                current_userspace!().write_val(arg, &winsize)?;
                Ok(0)
            }
            IoctlCmd::TIOCSWINSZ => {
                let winsize = current_userspace!().read_val(arg)?;
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
                current_userspace!().write_val(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
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
                current_userspace!().write_val(arg, &len)?;
                Ok(0)
            }
            _ => Ok(0),
        }
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

impl Pollable for PtySlave {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.master().slave_poll(mask, poller)
    }
}

impl FileIo for PtySlave {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut buf = vec![0u8; writer.avail()];
        self.job_control.wait_until_in_foreground()?;
        let read_len = self.master().output.read(&mut buf)?;
        writer.write_fallible(&mut buf.as_slice().into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buf = reader.collect()?;
        let write_len = buf.len();
        let master = self.master();
        for ch in buf {
            // do we need to add '\r' here?
            if ch == b'\n' {
                master.slave_push_char(b'\r');
                master.slave_push_char(b'\n');
            } else {
                master.slave_push_char(ch);
            }
        }
        Ok(write_len)
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
                current_userspace!().write_val(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCSPGRP => {
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
                current_userspace!().write_val(arg, &buffer_len)?;
                Ok(0)
            }
            _ => Ok(0),
        }
    }
}
