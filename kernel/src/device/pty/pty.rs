// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::task::Task;

use crate::{
    current_userspace,
    device::tty::line_discipline::LineDiscipline,
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
        broadcast_signal_async,
        posix_thread::{AsPosixThread, AsThreadLocal},
        signal::{PollHandle, Pollable, Pollee},
        JobControl, Terminal,
    },
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 4096;
const IO_CAPACITY: usize = 4096;

/// Pseudo terminal master.
/// Internally, it has two buffers.
/// One is inside ldisc, which is written by master and read by slave,
/// the other is a ring buffer, which is written by slave and read by master.
pub struct PtyMaster {
    ptmx: Arc<dyn Inode>,
    index: u32,
    slave: Arc<PtySlave>,
    input: SpinLock<RingBuffer<u8>>,
    pollee: Pollee,
}

impl PtyMaster {
    pub fn new(ptmx: Arc<dyn Inode>, index: u32) -> Arc<Self> {
        Arc::new_cyclic(move |master| {
            let slave = Arc::new_cyclic(move |weak_self| PtySlave {
                ldisc: SpinLock::new(LineDiscipline::new()),
                job_control: JobControl::new(),
                pollee: Pollee::new(),
                master: master.clone(),
                weak_self: weak_self.clone(),
            });

            PtyMaster {
                ptmx,
                index,
                slave,
                input: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
                pollee: Pollee::new(),
            }
        })
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn ptmx(&self) -> &Arc<dyn Inode> {
        &self.ptmx
    }

    pub fn slave(&self) -> &Arc<PtySlave> {
        &self.slave
    }

    fn slave_push(&self, chs: &[u8]) {
        let mut input = self.input.lock();

        for ch in chs {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the TTY driver implementation. See the ONLCR flag for more details.
            if *ch == b'\n' {
                input.push_overwrite(b'\r');
                input.push_overwrite(b'\n');
                continue;
            }
            input.push_overwrite(*ch);
        }
        self.pollee.notify(IoEvents::IN);
    }

    fn slave_echo(&self) -> impl FnMut(&str) + '_ {
        let mut input = self.input.lock();
        let mut has_notified = false;

        move |content| {
            for byte in content.as_bytes() {
                input.push_overwrite(*byte);
            }

            if !has_notified {
                self.pollee.notify(IoEvents::IN);
                has_notified = true;
            }
        }
    }

    fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut input = self.input.lock();
        if input.is_empty() {
            return_errno_with_message!(Errno::EAGAIN, "the buffer is empty");
        }

        let read_len = input.len().min(buf.len());
        input.pop_slice(&mut buf[..read_len]).unwrap();
        self.pollee.invalidate();

        Ok(read_len)
    }

    fn check_io_events(&self) -> IoEvents {
        let input = self.input.lock();

        if !input.is_empty() {
            IoEvents::IN | IoEvents::OUT
        } else {
            IoEvents::OUT
        }
    }
}

impl Pollable for PtyMaster {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileIo for PtyMaster {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for non-blocking mode and timeout
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let read_len = self.wait_events(IoEvents::IN, None, || self.try_read(&mut buf))?;

        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        self.slave.master_push(&buf[..write_len]);
        Ok(write_len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS
            | IoctlCmd::TCSETS
            | IoctlCmd::TIOCGPTN
            | IoctlCmd::TIOCGWINSZ
            | IoctlCmd::TIOCSWINSZ => return self.slave.ioctl(cmd, arg),
            IoctlCmd::TIOCSPTLCK => {
                // TODO: lock/unlock pty
            }
            IoctlCmd::TIOCGPTPEER => {
                let current_task = Task::current().unwrap();
                let posix_thread = current_task.as_posix_thread().unwrap();
                let thread_local = current_task.as_thread_local().unwrap();

                // TODO: deal with open options
                let slave = {
                    let slave_name = {
                        let devpts_path = super::DEV_PTS.get().unwrap().abs_path();
                        format!("{}/{}", devpts_path, self.index())
                    };

                    let fs_path = FsPath::try_from(slave_name.as_str())?;

                    let inode_handle = {
                        let fs = posix_thread.fs().resolver().read();
                        let flags = AccessMode::O_RDWR as u32;
                        let mode = (InodeMode::S_IRUSR | InodeMode::S_IWUSR).bits();
                        fs.open(&fs_path, flags, mode)?
                    };
                    Arc::new(inode_handle)
                };

                let fd = {
                    let file_table = thread_local.borrow_file_table();
                    let mut file_table_locked = file_table.unwrap().write();
                    // TODO: deal with the O_CLOEXEC flag
                    file_table_locked.insert(slave, FdFlags::empty())
                };
                return Ok(fd);
            }
            IoctlCmd::FIONREAD => {
                let len = self.input.lock().len() as i32;
                current_userspace!().write_val(arg, &len)?;
            }
            _ => (self.slave.clone() as Arc<dyn Terminal>).job_ioctl(cmd, arg, true)?,
        }

        Ok(0)
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
    ldisc: SpinLock<LineDiscipline>,
    job_control: JobControl,
    pollee: Pollee,
    master: Weak<PtyMaster>,
    weak_self: Weak<Self>,
}

impl PtySlave {
    pub fn index(&self) -> u32 {
        self.master().index()
    }

    fn master(&self) -> Arc<PtyMaster> {
        self.master.upgrade().unwrap()
    }

    fn master_push(&self, chs: &[u8]) {
        let mut ldisc = self.ldisc.lock();

        let master = self.master();
        let mut echo = master.slave_echo();

        for ch in chs {
            ldisc.push_char(
                *ch,
                |signum| {
                    if let Some(foreground) = self.job_control.foreground() {
                        broadcast_signal_async(Arc::downgrade(&foreground), signum);
                    }
                },
                &mut echo,
            );
        }
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

impl Device for PtySlave {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> crate::fs::device::DeviceId {
        DeviceId::new(88, self.index())
    }
}

impl Terminal for PtySlave {
    fn job_control(&self) -> &JobControl {
        &self.job_control
    }
}

impl Pollable for PtySlave {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileIo for PtySlave {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.job_control.wait_until_in_foreground()?;

        // TODO: Add support for non-blocking mode and timeout
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let read_len =
            self.wait_events(IoEvents::IN, None, || self.ldisc.lock().try_read(&mut buf))?;
        self.pollee.invalidate();

        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        self.master().slave_push(&buf[..write_len]);
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
            IoctlCmd::TIOCGPTN => {
                let idx = self.index();
                current_userspace!().write_val(arg, &idx)?;
            }
            IoctlCmd::TIOCGWINSZ => {
                let winsize = self.ldisc.lock().window_size();
                current_userspace!().write_val(arg, &winsize)?;
            }
            IoctlCmd::TIOCSWINSZ => {
                let winsize = current_userspace!().read_val(arg)?;
                self.ldisc.lock().set_window_size(winsize);
            }
            IoctlCmd::FIONREAD => {
                let buffer_len = self.ldisc.lock().buffer_len() as i32;
                current_userspace!().write_val(arg, &buffer_len)?;
            }
            _ => (self.weak_self.upgrade().unwrap() as Arc<dyn Terminal>)
                .job_ioctl(cmd, arg, false)?,
        }

        Ok(0)
    }
}
