Unwinding library in Rust and for Rust
======================================

[![crates.io](https://img.shields.io/crates/v/unwinding.svg)](https://crates.io/crates/unwinding)
[![docs.rs](https://docs.rs/unwinding/badge.svg)](https://docs.rs/unwinding)
[![license](https://img.shields.io/crates/l/unwinding.svg)](https://crates.io/crates/unwinding)

This library serves two purposes:
1. Provide a pure Rust alternative to libgcc_eh or libunwind.
2. Provide easier unwinding support for `#![no_std]` targets.

Currently supports x86_64, x86, RV64, RV32, AArch64, and LoongArch64.

## Unwinder

The unwinder can be enabled with `unwinder` feature. Here are the feature gates related to the unwinder:

| Feature              | Default | Description |
|--------------------- |---------|-|
| unwinder             | Yes     | The primary feature gate to enable the unwinder |
| fde-phdr-dl          | Yes     | Use `dl_iterator_phdr` to retrieve frame unwind table. Depends on libc. |
| fde-phdr-aux         | No      | Use ELF auxiliary vector to retrieve frame unwind table. Depends on libc. |
| fde-registry         | Yes     | Provide `__register__frame` and others for dynamic registration. Requires either `libc` or `spin` for a mutex implementation. |
| fde-gnu-eh-frame-hdr | No      | Use `__executable_start`, `__etext` and `__GNU_EH_FRAME_HDR` to retrieve frame unwind table. The former two symbols are usually provided by the linker, while the last one is provided if GNU LD is used and --eh-frame-hdr option is enabled. |
| fde-static           | No      | Use `__executable_start`, `__etext` and `__eh_frame` to retrieve frame unwind table. The former two symbols are usually provided by the linker, while the last one would need to be provided by the user via linker script.  |
| fde-custom           | No      | Allow the program to provide a custom means of retrieving frame unwind table at runtime via the `set_custom_eh_frame_finder` function. |
| dwarf-expr           | Yes     | Enable the dwarf expression evaluator. Usually not necessary for Rust |
| hide-trace           | Yes     | Hide unwinder frames in back trace |

If you want to use the unwinder for other Rust (C++, or any programs that utilize the unwinder), you can build the [`unwinding_dyn`](cdylib) crate provided, and use `LD_PRELOAD` to replace the system unwinder with it.
```sh
cd cdylib
cargo build --release
# Test the unwinder using rustc. Why not :)
LD_PRELOAD=`../target/release/libunwinding_dyn.so` rustc -Ztreat-err-as-bug
```

If you want to link to the unwinder in a Rust binary, simply add
```rust
extern crate unwinding;
```

## Personality and other utilities

The library also provides Rust personality function. This can work with the unwinder described above or with a different unwinder. This can be handy if you are working on a `#![no_std]` binary/staticlib/cdylib and you still want unwinding support.

Note that these features depend on nightly features.

Here are the feature gates related:

| Feature       | Default | Description |
|---------------|---------|-|
| personality   | No      | Provides `#[lang = eh_personality]` |
| print         | No      | Provides `(e)?print(ln)?`. This is really only here because panic handler needs to print things. Depends on libc. |
| panicking     | No      | Provides a generic `begin_panic` and `catch_unwind`. Only stack unwinding functionality is provided, memory allocation and panic handling is left to the user. |
| panic         | No      | Provides Rust `begin_panic` and `catch_unwind`. Only stack unwinding functionality is provided and no printing is done, because this feature does not depend on libc. |
| panic-handler | No      | Provides `#[panic_handler]`. Provides similar behaviour on panic to std, with `RUST_BACKTRACE` support as well. Stack trace won't have symbols though. Depends on libc. |
| system-alloc  | No      | Provides a global allocator which calls `malloc` and friends. Provided for convience. |

If you are writing a `#![no_std]` program, simply enable `personality`, `panic-handler` and `system-alloc` in addition to the defaults, you instantly obtains the ability to do unwinding! An example is given in the [here](test_crates/throw_and_catch/src/main.rs).

## Baremetal

To use this library for baremetal projects, disable default features and enable `unwinder`, `fde-static`, `personality`, `panic`. `dwarf-expr` and `hide-trace` are optional. Modify the linker script by
```ld
/* Inserting these two lines */
. = ALIGN(8);
PROVIDE(__eh_frame = .);
/* before .eh_frame rule */
.eh_frame : { KEEP (*(.eh_frame)) *(.eh_frame.*) }
```

And that's it! After you ensured that the global allocator is functional, you can use `unwinding::panic::begin_panic` to initiate an unwing and catch using `unwinding::panic::catch_unwind`, as if you have a `std`.

If your linker supports `--eh-frame-hdr` you can also try to use `fde-gnu-eh-frame-hdr` instead of `fde-static`. GNU LD will provides a `__GNU_EH_FRAME_HDR` magic symbol so you don't have to provide `__eh_frame` through linker script.

If you have your own version of `thread_local` and `println!` working, you can port [`panic_handler.rs`](src/panic_handler.rs) for double-panic protection and stack traces!
