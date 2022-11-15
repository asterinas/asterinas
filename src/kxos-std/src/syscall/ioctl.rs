use crate::fs::file::FileDescripter;
use crate::fs::ioctl::termio::KernelTermios;
use crate::fs::ioctl::IoctlCmd;
use crate::memory::read_val_from_user;
use crate::memory::write_val_to_user;
use crate::prelude::*;

use super::SyscallReturn;
use super::SYS_IOCTL;

pub fn sys_ioctl(fd: FileDescripter, cmd: u32, arg: Vaddr) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_IOCTL]", SYS_IOCTL);
    let ioctl_cmd = IoctlCmd::try_from(cmd)?;
    debug!(
        "fd = {}, ioctl_cmd = {:?}, arg = 0x{:x}",
        fd, ioctl_cmd, arg
    );
    match ioctl_cmd {
        IoctlCmd::TCGETS => {
            if fd == 0 || fd == 1 {
                let termio = KernelTermios::fake_kernel_termios();
                write_val_to_user(arg, &termio)?;
            } else {
                todo!()
            }
        }
        IoctlCmd::TIOCGPGRP => {
            // FIXME: Get the process group ID of the foreground process group on this terminal.
            // We currently only return the pgid of current process
            let current = current!();
            let pgid = current.pgid();
            write_val_to_user(arg, &pgid)?;
        }
        IoctlCmd::TIOCSPGRP => {
            let pgid = read_val_from_user::<i32>(arg)?;
            debug!("set foreground process group id: {}", pgid);
            // TODO: Set the foreground process group
        }
        IoctlCmd::TCSETS => {
            if fd == 0 || fd == 1 {
                let termio = read_val_from_user::<KernelTermios>(arg)?;
                debug!("termio = {:x?}", termio);
                // TODO: Set termios
            } else {
                todo!()
            }
        }
        IoctlCmd::TIOCGWINSZ => {
            // TODO:get window size
        }
        _ => todo!(),
    }
    Ok(SyscallReturn::Return(0))
}
