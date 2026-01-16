// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::task::Task;

use super::{PtySlave, driver::PtyDriver};
use crate::{
    device::tty::TtyFlags,
    events::IoEvents,
    fs::{
        devpts::Ptmx,
        file_table::FdFlags,
        inode_handle::FileIo,
        path::FsPath,
        utils::{AccessMode, InodeIo, OpenArgs, StatusFlags, mkmod},
    },
    prelude::*,
    process::{
        Terminal,
        posix_thread::AsThreadLocal,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::{RawIoctl, dispatch_ioctl},
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
    ptmx: Arc<Ptmx>,
    slave: Arc<PtySlave>,
}

impl PtyMaster {
    pub(super) fn new(ptmx: Arc<Ptmx>, index: u32) -> Box<Self> {
        let slave = PtySlave::new(index, PtyDriver::new());

        Box::new(Self { ptmx, slave })
    }

    pub(super) fn slave(&self) -> &Arc<PtySlave> {
        &self.slave
    }

    fn master_flags(&self) -> &TtyFlags {
        self.slave.driver().tty_flags()
    }

    fn slave_flags(&self) -> &TtyFlags {
        self.slave.tty_flags()
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.slave().driver().buffer_len() > 0 {
            events |= IoEvents::IN | IoEvents::RDNORM;
        }

        if self.slave().can_push() {
            events |= IoEvents::OUT;
        }

        if self.master_flags().is_other_closed() {
            events |= IoEvents::HUP;
        }

        // Deal with packet mode.
        let packet_ctrl = self.slave.driver().packet_ctrl();
        if packet_ctrl.has_status() {
            events |= IoEvents::PRI | IoEvents::IN | IoEvents::RDNORM;
        }

        events
    }
}

impl Pollable for PtyMaster {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.slave
            .driver()
            .pollee()
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl InodeIo for PtyMaster {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        // TODO: Add support for timeout.
        let mut buf = vec![0u8; writer.avail().min(IO_CAPACITY)];
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);
        let read_len = if is_nonblocking {
            self.slave.driver().try_read(&mut buf)?
        } else {
            self.wait_events(IoEvents::IN, None, || {
                self.slave.driver().try_read(&mut buf)
            })?
        };
        self.slave.driver().pollee().invalidate();
        self.slave.notify_output();

        // TODO: Confirm what we should do if `write_fallible` fails in the middle.
        writer.write_fallible(&mut buf[..read_len].into())?;
        Ok(read_len)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let mut buf = vec![0u8; reader.remain().min(IO_CAPACITY)];
        let write_len = reader.read_fallible(&mut buf.as_mut_slice().into())?;

        // TODO: Add support for timeout.
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);
        let len = if is_nonblocking {
            self.slave.push_input(&buf[..write_len])?
        } else {
            self.wait_events(IoEvents::OUT, None, || {
                self.slave.push_input(&buf[..write_len])
            })?
        };
        self.slave.driver().pollee().invalidate();
        Ok(len)
    }
}

mod ioctl_defs {
    use crate::util::ioctl::{InData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

    pub(super) type SetPtyLock   = ioc!(TIOCSPTLCK,  b'T', 0x31, InData<i32>);
    pub(super) type GetPtyLock   = ioc!(TIOCGPTLCK,  b'T', 0x39, OutData<i32>);

    pub(super) type OpenPtySlave = ioc!(TIOCGPTPEER, b'T', 0x41, NoData);

    pub(super) type SetPktMode   = ioc!(TIOCPKT,     0x5420,     InData<i32>);
    pub(super) type GetPktMode   = ioc!(TIOCGPKT,    b'T', 0x38, OutData<i32>);
}

impl FileIo for PtyMaster {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a pty");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        use crate::{device::tty::ioctl_defs::*, fs::utils::ioctl_defs::GetNumBytesToRead};

        dispatch_ioctl!(match raw_ioctl {
            GetTermios | SetTermios | SetTermiosWait | SetTermiosFlush | GetWinSize
            | SetWinSize | GetPtyNumber => {
                return self.slave.ioctl(raw_ioctl);
            }

            cmd @ SetPtyLock => {
                let should_lock = cmd.read()? != 0;

                let flags = self.master_flags();
                if should_lock {
                    flags.set_pty_locked();
                } else {
                    flags.clear_pty_locked();
                }
            }
            cmd @ GetPtyLock => {
                let is_locked = if self.master_flags().is_pty_locked() {
                    1
                } else {
                    0
                };

                cmd.write(&is_locked)?;
            }
            _cmd @ OpenPtySlave => {
                let current_task = Task::current().unwrap();
                let thread_local = current_task.as_thread_local().unwrap();

                // TODO: Deal with `open()` flags.
                let slave = {
                    let fs_ref = thread_local.borrow_fs();
                    let path_resolver = fs_ref.resolver().read();

                    let slave_name = {
                        let devpts_path = path_resolver
                            .make_abs_path(super::DEV_PTS.get().unwrap())
                            .into_string();
                        format!("{}/{}", devpts_path, self.slave.index())
                    };

                    let fs_path = FsPath::try_from(slave_name.as_str())?;

                    let inode_handle = {
                        let open_args = OpenArgs::from_modes(AccessMode::O_RDWR, mkmod!(u+rw));
                        path_resolver.lookup(&fs_path)?.open(open_args)?
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
            cmd @ GetNumBytesToRead => {
                let len = self.slave.driver().buffer_len() as i32;

                cmd.write(&len)?;
            }
            cmd @ SetPktMode => {
                let new_mode = cmd.read()? != 0;

                self.slave.driver().packet_ctrl().set_mode(new_mode);
            }
            cmd @ GetPktMode => {
                let packet_mode = if self.slave.driver().packet_ctrl().mode() {
                    1
                } else {
                    0
                };

                cmd.write(&packet_mode)?;
            }

            _ => (self.slave.clone() as Arc<dyn Terminal>).job_ioctl(raw_ioctl, true)?,
        });

        Ok(0)
    }
}

impl Drop for PtyMaster {
    fn drop(&mut self) {
        if let Some(devpts) = self.ptmx.devpts() {
            let index = self.slave.index();
            devpts.remove_slave(index);
        }

        self.slave_flags().set_other_closed();
        self.slave.notify_hup();
    }
}
