// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;
use aster_frame::vm::VmIo;
pub mod net;

/// copy bytes from user space of current process. The bytes len is the len of dest.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_bytes(src, dest)?)
}

/// copy val (Plain of Data type) from user space of current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> Result<T> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_val(src)?)
}

/// write bytes from user space of current process. The bytes len is the len of src.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_bytes(dest, src)?)
}

/// write val (Plain of Data type) to user space of current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_val(dest, val)?)
}

/// read a cstring from user, the length of cstring should not exceed max_len(include null byte)
pub fn read_cstring_from_user(addr: Vaddr, max_len: usize) -> Result<CString> {
    let mut buffer = vec![0u8; max_len];
    read_bytes_from_user(addr, &mut buffer)?;
    Ok(CString::from(CStr::from_bytes_until_nul(&buffer)?))
}
