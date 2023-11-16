cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod x86_64;
        pub use x86_64::*;
    } else if #[cfg(target_arch = "x86")] {
        mod i386;
        pub use i386::*;
    } else {
        compile_error!("Unsupported target_arch");
    }
}
