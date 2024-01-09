use crate::prelude::*;
use aster_frame::vm::VmIo;
pub mod net;

/// Read bytes from user space of current process. The read len is the len of `dest`. This
/// method will return error if short read occurs.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_bytes(src, dest)?)
}

/// Read value of `Pod` type from user space of current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> Result<T> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_val(src)?)
}

/// Write bytes from user space of current process. The write len is the len of `src`.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_bytes(dest, src)?)
}

/// Write value of `Pod` type to user space of current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_val(dest, val)?)
}

/// Read a cstring from user space, the length of cstring should not exceed max_len(include null byte).
/// Further, the whole string should be in exact one mapping, it cannot cross the mapping boundary.
///
/// FIXME: should we allow that a string can cross the mapping boundary?
pub fn read_cstring_from_user(addr: Vaddr, max_len: usize) -> Result<CString> {
    let vm_mapping = {
        let current = current!();
        let root_vmar = current.root_vmar();
        root_vmar.get_vm_mapping(addr)?
    };

    let max_read_len = {
        let max_len_in_mapping = vm_mapping.map_to_addr() + vm_mapping.map_size() - addr;
        max_len.min(max_len_in_mapping)
    };

    let mut buffer = vec![0u8; max_read_len];
    vm_mapping.read_bytes(addr - vm_mapping.map_to_addr(), &mut buffer)?;
    Ok(CString::from(CStr::from_bytes_until_nul(&buffer)?))
}
