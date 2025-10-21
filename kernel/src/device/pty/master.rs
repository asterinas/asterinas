// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::task::Task;

use super::PtySlave;
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        devpts::DevPts,
        file_table::FdFlags,
        fs_resolver::FsPath,
        inode_handle::FileIo,
        open_args::OpenArgs,
        utils::{mkmod, AccessMode, Inode, IoctlCmd},
    },
    prelude::*,
    process::{
        posix_thread::AsThreadLocal,
        signal::{PollHandle, Pollable},
        Terminal,
    },
};

const IO_CAPACITY: usize = 4096;

/// A pseudoterminal master.
///
/// A pseudoterminal contains two buffers:
///  * The input buffer is written by the master and read by the slave, which is maintained in the
///    line discipline (part of [`PtySlave`], which is a [`Tty`]).
///  * The output buffer is written by the slave and read by the master, which is maintained in the
///    driver (i.e., [`PtyDriver`]).
///
/// [`Tty`]: crate::device::tty::Tty
pub struct PtyMaster {
    ptmx: Arc<dyn Inode>,
    slave: Arc<PtySlave>,
}

impl PtyMaster {
    pub(super) fn new(ptmx: Arc<dyn Inode>, index: u32) -> Arc<Self> {
        let slave = PtySlave::new(index);

        Arc::new(Self { ptmx, slave })
    }

    pub(super) fn slave(&self) -> &Arc<PtySlave> {
        &self.slave
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.slave.as_tty().driver().buffer_len() > 0 {
            events |= IoEvents::IN;
        }

        if self.slave.as_tty().can_push() {
            events |= IoEvents::OUT;
        }

        events
    }
}

impl Pollable for PtyMaster {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.slave()
            .as_tty()
            .driver()
            .pollee()
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileIo for PtyMaster {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for non-blocking mode and timeout
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let read_len = self.wait_events(IoEvents::IN, None, || {
            self.slave().as_tty().driver().try_read(&mut buf)
        })?;
        self.slave().as_tty().driver().pollee().invalidate();
        self.slave().as_tty().notify_output();

        // TODO: Confirm what we should do if `write_fallible` fails in the middle.
        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        // TODO: Add support for non-blocking mode and timeout
        let len = self.wait_events(IoEvents::OUT, None, || {
            Ok(self.slave().as_tty().push_input(&buf[..write_len])?)
        })?;
        self.slave().as_tty().driver().pollee().invalidate();
        Ok(len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS
            | IoctlCmd::TCSETS
            | IoctlCmd::TCSETSW
            | IoctlCmd::TCSETSF
            | IoctlCmd::TIOCGWINSZ
            | IoctlCmd::TIOCSWINSZ
            | IoctlCmd::TIOCGPTN => return self.slave().ioctl(cmd, arg),
            IoctlCmd::TIOCSPTLCK => {
                // TODO: Lock or unlock the PTY.
            }
            IoctlCmd::TIOCGPTPEER => {
                let current_task = Task::current().unwrap();
                let thread_local = current_task.as_thread_local().unwrap();

                // TODO: Deal with `open()` flags.
                let slave = {
                    let slave_name = {
                        let devpts_path = super::DEV_PTS.get().unwrap().abs_path();
                        format!("{}/{}", devpts_path, self.slave().as_tty().index())
                    };

                    let fs_path = FsPath::try_from(slave_name.as_str())?;

                    let inode_handle = {
                        let open_args = OpenArgs::from_modes(AccessMode::O_RDWR, mkmod!(u+rw));
                        thread_local
                            .borrow_fs()
                            .resolver()
                            .read()
                            .lookup(&fs_path)?
                            .open(open_args)?
                    };
                    Arc::new(inode_handle)
                };

                let fd = {
                    let file_table = thread_local.borrow_file_table();
                    let mut file_table_locked = file_table.unwrap().write();
                    // TODO: Deal with the `O_CLOEXEC` flag.
                    file_table_locked.insert(slave, FdFlags::empty())
                };
                return Ok(fd);
            }
            IoctlCmd::FIONREAD => {
                let len = self.slave().as_tty().driver().buffer_len() as i32;
                current_userspace!().write_val(arg, &len)?;
            }
            _ => (self.slave().as_tty().clone() as Arc<dyn Terminal>).job_ioctl(cmd, arg, true)?,
        }

        Ok(0)
    }
}

impl Drop for PtyMaster {
    fn drop(&mut self) {
        let fs = self.ptmx.fs();
        let devpts = fs.downcast_ref::<DevPts>().unwrap();

        let index = self.slave().as_tty().index();
        devpts.remove_slave(index);
    }
}
