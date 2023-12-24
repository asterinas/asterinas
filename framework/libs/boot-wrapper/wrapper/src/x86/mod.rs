cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod amd64_efi;
    } else if #[cfg(target_arch = "x86")] {
        mod legacy_i386;
    } else {
        compile_error!("Unsupported target_arch");
    }
}

pub mod paging;
pub mod relocation;
