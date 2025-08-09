// SPDX-License-Identifier: MPL-2.0

use ostd::mm::PAGE_SIZE;

// FIXME: define Linux-compatible vDSO VMO layout. Here we temporarily use those
// in RISC-V.
pub const VDSO_DATA_OFFSET: usize = 0x0;
pub const VDSO_DATA_SIZE: usize = PAGE_SIZE;
pub const VDSO_TEXT_OFFSET: usize = VDSO_DATA_OFFSET + VDSO_DATA_SIZE + PAGE_SIZE;
pub const VDSO_TEXT_SIZE: usize = PAGE_SIZE;
pub const VDSO_VMO_SIZE: usize = VDSO_TEXT_OFFSET + VDSO_TEXT_SIZE;
