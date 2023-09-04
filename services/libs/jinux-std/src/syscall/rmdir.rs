use crate::fs::fs_resolver::FsPath;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_RMDIR;

pub fn sys_rmdir(pathname_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RMDIR);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!("pathname = {:?}", pathname);

    let current = current!();
    let (dir_dentry, name) = {
        let pathname = pathname.to_string_lossy();
        let fs_path = FsPath::try_from(pathname.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentry.rmdir(name.trim_end_matches('/'))?;
    Ok(SyscallReturn::Return(0))
}
