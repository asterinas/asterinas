// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use super::SyscallReturn;
use crate::prelude::*;

// We don't use the real name and version of our os here. Instead, we pick up fake values witch is the same as the ones of linux.
// The values are used to fool glibc since glibc will check the version and os name.
static UTS_NAME: Once<UtsName> = Once::new();

const UTS_FIELD_LEN: usize = 65;

#[derive(Debug, Clone, Copy, Pod)]
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

pub(super) fn init() {
    UTS_NAME.call_once(|| {
        let copy_slice = |src: &[u8], dst: &mut [u8]| {
            let len = src.len().min(dst.len());
            dst[..len].copy_from_slice(&src[..len]);
        };

        let mut uts_name = UtsName::new();
        copy_slice(b"Linux", &mut uts_name.sysname);
        copy_slice(b"WHITLEY", &mut uts_name.nodename);
        copy_slice(b"5.13.0", &mut uts_name.release);
        copy_slice(b"5.13.0", &mut uts_name.version);
        copy_slice(b"x86_64", &mut uts_name.machine);
        copy_slice(b"", &mut uts_name.domainname);

        uts_name
    });
}

pub fn sys_uname(old_uname_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("old uname addr = 0x{:x}", old_uname_addr);
    ctx.user_space()
        .write_val(old_uname_addr, UTS_NAME.get().unwrap())?;
    Ok(SyscallReturn::Return(0))
}
