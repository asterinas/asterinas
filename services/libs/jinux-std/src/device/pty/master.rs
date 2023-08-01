use crate::{
    fs::{
        file_handle::FileLike,
        fs_resolver::FsPath,
        utils::{AccessMode, Inode, InodeMode, IoEvents, IoctlCmd, Poller},
    },
    prelude::*,
    util::{read_val_from_user, write_val_to_user},
};
use alloc::format;
use jinux_frame::sync::SpinLock;
use ringbuf::{ring_buffer::RbBase, HeapRb, Rb};

use crate::{device::tty::line_discipline::LineDiscipline, fs::utils::Pollee};

use super::slave::PtySlave;

const PTS_DIR: &str = "/dev/pts";
const BUFFER_CAPACITY: usize = 4096;

/// Pesudo terminal master.
/// Internally, it has two buffers.
/// One is inside ldisc, which is written by master and read by slave,
/// the other is a ring buffer, which is written by slave and read by master.
pub struct PtyMaster {
    ptmx: Arc<dyn Inode>,
    index: usize,
    ldisc: LineDiscipline,
    master_buffer: SpinLock<HeapRb<u8>>,
    /// The state of master buffer
    pollee: Pollee,
}

impl PtyMaster {
    pub fn new_pair(index: u32, ptmx: Arc<dyn Inode>) -> Result<(Arc<PtyMaster>, Arc<PtySlave>)> {
        debug!("allocate pty index = {}", index);
        let master = Arc::new(PtyMaster {
            ptmx,
            index: index as usize,
            master_buffer: SpinLock::new(HeapRb::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(IoEvents::OUT),
            ldisc: LineDiscipline::new(),
        });
        let slave = Arc::new(PtySlave::new(master.clone()));
        Ok((master, slave))
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn ptmx(&self) -> &Arc<dyn Inode> {
        &self.ptmx
    }

    pub(super) fn slave_push_char(&self, item: u8) -> Result<()> {
        let mut buf = self.master_buffer.lock_irq_disabled();
        if buf.is_full() {
            return_errno_with_message!(Errno::EIO, "the buffer is full");
        }
        // Unwrap safety: the buf is not full, so push will always succeed.
        buf.push(item).unwrap();
        self.update_state(&buf);
        Ok(())
    }

    pub(super) fn slave_read(&self, buf: &mut [u8]) -> Result<usize> {
        self.ldisc.read(buf)
    }

    pub(super) fn slave_poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let poll_out_mask = mask & IoEvents::OUT;
        let poll_in_mask = mask & IoEvents::IN;

        loop {
            let mut poll_status = IoEvents::empty();

            if !poll_in_mask.is_empty() {
                let poll_in_status = self.ldisc.poll(poll_in_mask, poller);
                poll_status |= poll_in_status;
            }

            if !poll_out_mask.is_empty() {
                let poll_out_status = self.pollee.poll(poll_out_mask, poller);
                poll_status |= poll_out_status;
            }

            if !poll_status.is_empty() || poller.is_none() {
                return poll_status;
            }

            poller.unwrap().wait();
        }
    }

    fn update_state(&self, buf: &HeapRb<u8>) {
        if buf.is_full() {
            self.pollee.del_events(IoEvents::OUT);
        } else {
            self.pollee.add_events(IoEvents::OUT);
        }

        if buf.is_empty() {
            self.pollee.del_events(IoEvents::IN)
        } else {
            self.pollee.add_events(IoEvents::IN);
        }
    }
}

impl FileLike for PtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // TODO: deal with nonblocking read
        if buf.len() == 0 {
            return Ok(0);
        }

        let poller = Poller::new();
        loop {
            let mut master_buf = self.master_buffer.lock_irq_disabled();

            if master_buf.is_empty() {
                self.update_state(&master_buf);
                let events = self.pollee.poll(IoEvents::IN, Some(&poller));
                if !events.contains(IoEvents::IN) {
                    drop(master_buf);
                    poller.wait();
                }
                continue;
            }

            let read_len = master_buf.len().min(buf.len());
            master_buf.pop_slice(&mut buf[..read_len]);
            self.update_state(&master_buf);
            return Ok(read_len);
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut master_buf = self.master_buffer.lock();

        if self.ldisc.termios().contain_echo() && master_buf.len() + buf.len() > BUFFER_CAPACITY {
            return_errno_with_message!(
                Errno::EIO,
                "the written bytes exceeds the master buf capacity"
            );
        }

        for item in buf {
            self.ldisc.push_char(*item, |content| {
                for byte in content.as_bytes() {
                    // Unwrap safety: the master buf is ensured to have enough space.
                    master_buf.push(*byte).unwrap();
                }
            });
        }

        self.update_state(&master_buf);
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS => {
                let termios = self.ldisc.termios();
                write_val_to_user(arg, &termios)?;
                Ok(0)
            }
            IoctlCmd::TCSETS => {
                let termios = read_val_from_user(arg)?;
                self.ldisc.set_termios(termios);
                Ok(0)
            }
            IoctlCmd::TIOCSPTLCK => {
                // TODO: lock/unlock pty
                Ok(0)
            }
            IoctlCmd::TIOCGPTN => {
                let idx = self.index() as u32;
                write_val_to_user(arg, &idx)?;
                Ok(0)
            }
            IoctlCmd::TIOCGPTPEER => {
                let current = current!();

                // TODO: deal with open options
                let slave = {
                    let slave_name = format!("{}/{}", PTS_DIR, self.index());
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
            IoctlCmd::TIOCGWINSZ => Ok(0),
            IoctlCmd::TIOCSCTTY => {
                // TODO
                let foreground = {
                    let current = current!();
                    let process_group = current.process_group().lock();
                    process_group.clone()
                };
                self.ldisc.set_fg(foreground);
                Ok(0)
            }
            IoctlCmd::TIOCGPGRP => {
                let Some(fg_pgid) = self.ldisc.fg_pgid() else {
                    return_errno_with_message!(
                        Errno::ESRCH,
                        "the foreground process group does not exist"
                    );
                };
                write_val_to_user(arg, &fg_pgid)?;
                Ok(0)
            }
            IoctlCmd::TIOCNOTTY => {
                self.ldisc.set_fg(Weak::new());
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let poll_out_mask = mask & IoEvents::OUT;
        let poll_in_mask = mask & IoEvents::IN;

        loop {
            let _master_buf = self.master_buffer.lock_irq_disabled();

            let mut poll_status = IoEvents::empty();

            if !poll_in_mask.is_empty() {
                let poll_in_status = self.pollee.poll(poll_in_mask, poller);
                poll_status |= poll_in_status;
            }

            if !poll_out_mask.is_empty() {
                let poll_out_status = self.ldisc.poll(poll_out_mask, poller);
                poll_status |= poll_out_status;
            }

            if !poll_status.is_empty() || poller.is_none() {
                return poll_status;
            }

            poller.unwrap().wait();
        }
    }
}
