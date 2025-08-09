// SPDX-License-Identifier: MPL-2.0

use ostd::mm::PAGE_SIZE;

pub const VDSO_DATA_OFFSET: usize = 0x80;
pub const VDSO_DATA_SIZE: usize = PAGE_SIZE;
pub const VDSO_TEXT_OFFSET: usize = 4 * PAGE_SIZE;
pub const VDSO_TEXT_SIZE: usize = PAGE_SIZE;
pub const VDSO_VMO_SIZE: usize = VDSO_TEXT_OFFSET + VDSO_TEXT_SIZE;
