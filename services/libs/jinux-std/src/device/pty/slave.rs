use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::fs::file_handle::FileLike;
use crate::fs::utils::{IoEvents, IoctlCmd, Poller};
use crate::prelude::*;
use crate::util::write_val_to_user;

use super::master::PtyMaster;

pub struct PtySlave(Arc<PtyMaster>);

impl PtySlave {
    pub fn new(master: Arc<PtyMaster>) -> Self {
        PtySlave(master)
    }

    pub fn index(&self) -> usize {
        self.0.index()
    }
}

impl Device for PtySlave {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> crate::fs::device::DeviceId {
        DeviceId::new(88, self.index() as u32)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.slave_read(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        for ch in buf {
            // do we need to add '\r' here?
            if *ch == b'\n' {
                self.0.slave_push_char(b'\r')?;
                self.0.slave_push_char(b'\n')?;
            } else {
                self.0.slave_push_char(*ch)?;
            }
        }
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TCGETS
            | IoctlCmd::TCSETS
            | IoctlCmd::TIOCGPGRP
            | IoctlCmd::TIOCGPTN
            | IoctlCmd::TIOCGWINSZ
            | IoctlCmd::TIOCSWINSZ => self.0.ioctl(cmd, arg),
            IoctlCmd::TIOCSCTTY => {
                // TODO:
                Ok(0)
            }
            IoctlCmd::TIOCNOTTY => {
                // TODO:
                Ok(0)
            }
            IoctlCmd::FIONREAD => {
                let buffer_len = self.0.slave_buf_len() as i32;
                write_val_to_user(arg, &buffer_len)?;
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.0.slave_poll(mask, poller)
    }
}
