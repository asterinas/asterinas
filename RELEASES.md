# Version 0.16.0 (2025-08-04)

This release introduces initial support for the **LoongArch CPU architecture**, a major milestone for the project. Version 0.16.0 also significantly expands our Linux ABI compatibility with the addition of **nine new system calls** such as `memfd_create` and `pidfd_open`.

Key enhancements include expanded functionality for **UNIX sockets (file descriptor passing and the `SOCK_SEQPACKET` socket type)**, partial support for **netlink sockets of the `NETLINK_KOBJECT_UEVENT` type**, the initial implementation of **CgroupFS**, and a major testing improvement with the integration of system call tests from the **Linux Test Project (LTP)**. We've also adopted **[Nix](https://nix.dev/manual/nix/2.28/introduction)** for building the initramfs, streamlining our cross-compilation and testing workflow.

## Asterinas Kernel

We have made the following key changes to the Asterinas kernel:

* New system calls or features:
    * Memory:
        * [Add the `mremap` system call](https://github.com/asterinas/asterinas/pull/2162)
        * [Add the `msync` system call based on an inefficient implementation](https://github.com/asterinas/asterinas/pull/2154)
        * [Add the `memfd_create` system call](https://github.com/asterinas/asterinas/pull/2149)
    * Processes and IPC:
        * [Add the `pidfd_open` system call along with the `CLONE_PIDFD` flag](https://github.com/asterinas/asterinas/pull/2151)
    * File systems and I/O in general:
        * [Add the `close_range` system call](https://github.com/asterinas/asterinas/pull/2128)
        * [Add the `fadvise64` system call (dummy implementation)](https://github.com/asterinas/asterinas/pull/2125)
        * [Add the `ioprio_get` and `ioprio_set` system calls (dummy implementation)](https://github.com/asterinas/asterinas/pull/2126)
        * [Add the `epoll_pwait2` system call](https://github.com/asterinas/asterinas/pull/2123)
* Enhanced system calls or features:
    * Processes:
        * [Add `FUTEX_WAKE_OP` support for the `futex` system call](https://github.com/asterinas/asterinas/pull/2146)
        * [Add `WSTOPPED` and `WCONTINUED` support to the `wait4` and `waitpid` system calls](https://github.com/asterinas/asterinas/pull/2166)
        * [Add more fields in `/proc/*/stat` and `/proc/*/status`](https://github.com/asterinas/asterinas/pull/2215)
    * File systems and I/O in general:
        * [Add a few more features for the `statx` system call](https://github.com/asterinas/asterinas/pull/2127)
        * [Fix partial writes and reads in writev and readv](https://github.com/asterinas/asterinas/pull/2230)
        * [Introduce `FsType` and `FsRegistry`](https://github.com/asterinas/asterinas/pull/2267)
    * Sockets and network:
        * [Enable UNIX sockets to send and receive file descriptors](https://github.com/asterinas/asterinas/pull/2176)
        * [Support `SO_PASSCRED` & `SCM_CREDENTIALS` & `SOCK_SEQPACKET` for UNIX sockets](https://github.com/asterinas/asterinas/pull/2268)
        * [Add `NETLINK_KOBJECT_UEVENT` support for netlink sockets (a partial implementation)](https://github.com/asterinas/asterinas/pull/2109)
        * [Support some missing socket options for UNIX stream sockets](https://github.com/asterinas/asterinas/pull/2192)
        * [Truncate netlink messages when the user-space buffer is full](https://github.com/asterinas/asterinas/pull/2155)
        * [Fix the networking address reusing behavior (`SO_REUSEADDR`)](https://github.com/asterinas/asterinas/pull/2277)
    * Security:
        * [Add basic cgroupfs implementation](https://github.com/asterinas/asterinas/pull/2121)
* New device support:
    * [Add basic i8042 keyboard support](https://github.com/asterinas/asterinas/pull/2054)
* Enhanced device support:
    * TTY
        * [Refactor the TTY abstraction to support multiple I/O devices correctly](https://github.com/asterinas/asterinas/pull/2108)
        * [Enhance the framebuffer console to support ANSI escape sequences](https://github.com/asterinas/asterinas/pull/2210)
* Test infrastructure:
    * [Introduce the system call tests from LTP](https://github.com/asterinas/asterinas/pull/2053)
    * [Use Nix to build initramfs](https://github.com/asterinas/asterinas/pull/2101)

## OSTD & OSDK

We have made the following key changes to OSTD:

* CPU architectures:
    * x86-64:
        * [Refactor floating-point context management in context switching and signal handling](https://github.com/asterinas/asterinas/pull/2219)
        * [Use iret instead of sysret if the context is not clean](https://github.com/asterinas/asterinas/pull/2271)
        * [Don't treat APIC IDs as CPU IDs](https://github.com/asterinas/asterinas/pull/2091)
        * [Fix some CPUID problems and add support for AMD CPUs](https://github.com/asterinas/asterinas/pull/2273)
    * RISC-V:
        * [Add RISC-V timer support](https://github.com/asterinas/asterinas/pull/2044)
        * [Parse device tree for RISC-V ISA extensions](https://github.com/asterinas/asterinas/pull/2113)
    * LoongArch:
        * [Add the initial LoongArch support](https://github.com/asterinas/asterinas/pull/2260)
* CPU:
    * [Add support for dynamically‌-allocated CPU-local objects](https://github.com/asterinas/asterinas/pull/2036)
    * [Require `T: Send` for `CpuLocal<T, S>`](https://github.com/asterinas/asterinas/pull/2171)
* Memory management:
    * [Adopt a two-phase locking scheme for page tables](https://github.com/asterinas/asterinas/pull/1948)
* Trap handling:
    * [Create `IrqChip` abstraction](https://github.com/asterinas/asterinas/pull/2107)
* Task and scheduling:
    * [Rewrite the Rust doc of OSTD's scheduling module](https://github.com/asterinas/asterinas/pull/2284)
    * [Fix the race between enabling IRQs and halting CPU](https://github.com/asterinas/asterinas/pull/2052)
* Test infrastructure:
    * [Add CI to check documentation and publish API documentation to a self-host website](https://github.com/asterinas/asterinas/pull/2218)

We have made the following key changes to OSDK:

* [Add OSDK's code coverage feature](https://github.com/asterinas/asterinas/pull/2203)
* [Support `cargo osdk test` for RISC-V](https://github.com/asterinas/asterinas/pull/2168)

# Before 0.16.0

Release notes were not kept for versions prior to 0.16.0.
