// ! Kernel symbol table support.
//!
//! This module provides functionality to initialize and dump the kernel symbol table
//! using the `ksym-bin` crate.

use alloc::string::String;

use ksym_bin::KallsymsMapped;
pub use ksym_bin::KSYM_NAME_LEN;
use spin::Once;

pub(crate) static KSYM: Once<KallsymsMapped<'static>> = Once::new();
extern "C" {
    fn __stext();
    fn __etext();
}

/// Initialize the kernel symbol table from the given ksym data blob.
pub fn init_ksym(ksym_data: &'static [u8]) {
    let kallsyms = KallsymsMapped::from_blob(ksym_data, __stext as u64, __etext as u64)
        .expect("Failed to parse ksym data");
    KSYM.call_once(|| kallsyms);
}

/// Dumps all kernel symbols as a string.
pub fn dump_ksyms() -> Option<String> {
    if let Some(ksym) = KSYM.get() {
        return Some(ksym.dump_all_symbols());
    }
    None
}

/// Looks up a kernel symbol by its address.
pub fn lookup_address(
    addr: u64,
    name_buf: &mut [u8; KSYM_NAME_LEN],
) -> Option<(&str, u64, u64, char)> {
    if let Some(ksym) = KSYM.get() {
        return ksym.lookup_address(addr, name_buf);
    }
    None
}

/// Looks up the address of a kernel symbol by its name.
pub fn lookup_name(name: &str) -> Option<u64> {
    if let Some(ksym) = KSYM.get() {
        return ksym.lookup_name(name);
    }
    None
}
