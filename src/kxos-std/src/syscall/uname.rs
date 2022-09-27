use core::ffi::CStr;

use alloc::ffi::CString;
use kxos_frame::{debug, vm::Vaddr};
use lazy_static::lazy_static;

use crate::{
    memory::write_bytes_to_user,
    syscall::{SyscallResult, SYS_UNAME},
};

lazy_static! {
    /// used to fool glibc
    static ref SYS_NAME: CString = CString::new("Linux").unwrap();
    static ref NODE_NAME: CString = CString::new("WHITLEY").unwrap();
    static ref RELEASE: CString = CString::new("5.13.0").unwrap();
    static ref VERSION: CString = CString::new("5.13.0").unwrap();
    static ref MACHINE: CString = CString::new("x86_64").unwrap();
    static ref DOMAIN_NAME: CString = CString::new("").unwrap();
    static ref UTS_NAME: UtsName = {
        let mut uts_name = UtsName::new();
        copy_cstring_to_u8_slice(&SYS_NAME, &mut uts_name.sysname);
        copy_cstring_to_u8_slice(&NODE_NAME, &mut uts_name.nodename);
        copy_cstring_to_u8_slice(&RELEASE, &mut uts_name.release);
        copy_cstring_to_u8_slice(&VERSION, &mut uts_name.version);
        copy_cstring_to_u8_slice(&MACHINE, &mut uts_name.machine);
        copy_cstring_to_u8_slice(&DOMAIN_NAME, &mut uts_name.domainname);
        uts_name
    };
}

const UTS_FIELD_LEN: usize = 65;

#[repr(C)]
struct UtsName {
    sysname: [u8; UTS_FIELD_LEN],
    nodename: [u8; UTS_FIELD_LEN],
    release: [u8; UTS_FIELD_LEN],
    version: [u8; UTS_FIELD_LEN],
    machine: [u8; UTS_FIELD_LEN],
    domainname: [u8; UTS_FIELD_LEN],
}

impl UtsName {
    const fn new() -> Self {
        UtsName {
            sysname: [0; UTS_FIELD_LEN],
            nodename: [0; UTS_FIELD_LEN],
            release: [0; UTS_FIELD_LEN],
            version: [0; UTS_FIELD_LEN],
            machine: [0; UTS_FIELD_LEN],
            domainname: [0; UTS_FIELD_LEN],
        }
    }
}

fn copy_cstring_to_u8_slice(src: &CStr, dst: &mut [u8]) {
    let src = src.to_bytes_with_nul();
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
}

pub fn sys_uname(old_uname_addr: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_UNAME]", SYS_UNAME);
    do_sys_uname(old_uname_addr as Vaddr);
    SyscallResult::Return(0)
}

pub fn do_sys_uname(old_uname_addr: Vaddr) -> usize {
    debug!("old_uname_addr: 0x{:x}", old_uname_addr);
    debug!("uts name size: {}", core::mem::size_of::<UtsName>());
    debug!("uts name align: {}", core::mem::align_of::<UtsName>());

    write_bytes_to_user(old_uname_addr, &UTS_NAME.sysname);
    write_bytes_to_user(old_uname_addr + UTS_FIELD_LEN, &UTS_NAME.nodename);
    write_bytes_to_user(old_uname_addr + UTS_FIELD_LEN * 2, &UTS_NAME.release);
    write_bytes_to_user(old_uname_addr + UTS_FIELD_LEN * 3, &UTS_NAME.version);
    write_bytes_to_user(old_uname_addr + UTS_FIELD_LEN * 4, &UTS_NAME.machine);
    write_bytes_to_user(old_uname_addr + UTS_FIELD_LEN * 5, &UTS_NAME.domainname);
    0
}
