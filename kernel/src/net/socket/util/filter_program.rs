// SPDX-License-Identifier: MPL-2.0

use crate::{current_userspace, prelude::*};

/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/filter.h#L24>.
// FIXME: We should define suitable Rust type instead of using the C type inside `FilterProgram`.
#[derive(Clone, Copy, Debug, Pod)]
#[repr(C)]
struct CSockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[expect(dead_code)]
#[derive(Debug, Clone)]
pub struct FilterProgram(Arc<[CSockFilter]>);

impl FilterProgram {
    pub fn read_from_user(addr: Vaddr, count: usize) -> Result<Self> {
        let mut filters = Vec::with_capacity(count);

        for i in 0..count {
            let addr = addr + i * core::mem::size_of::<CSockFilter>();
            let sock_filter = current_userspace!().read_val::<CSockFilter>(addr)?;
            filters.push(sock_filter);
        }

        Ok(Self(filters.into()))
    }
}
