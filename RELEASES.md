# Version 0.17.0 (2025-12-17)

This release marks a significant milestone in the evolution of Asterinas as we transition from "just a kernel" to a more complete, usable system. The headline of this release is the introduction of **Asterinas NixOS**, our first distribution that integrates the Asterinas kernel with the NixOS userspace, and enables real applications and services out of the box—including **XFCE**, **Podman**, and **systemd**. To support this growth, we are also strengthening our governance by establishing **the formal RFC (Request for Comments) process**, ensuring that major architectural decisions—starting with Asterinas NixOS itself—are designed transparently and collaboratively.

On the architecture front, **RISC-V support** has improved dramatically, with support for SMP (Symmetric Multiprocessing), FPU (Floating-Point Unit), VirtIO, and the SiFive HiFive Unleashed QEMU machine type. The kernel has also expanded with a new **input subsystem** (supporting keyboards and mice) and initial support for **namespaces and cgroups**. For filesystem developers, the big news is the addition of an **FS event notification mechanism (`inotify`)**, a new **ioctl infrastructure**, and a new filesystem type, **ConfigFS**. Finally, we are introducing **`sctrace`**—a dedicated tool for tracing and debugging syscall compatibility—now published on [crates.io](https://crates.io/crates/sctrace).

## Asterinas NixOS

We have made the following key changes to Asterinas NixOS:

* [Add NixOS as a distribution for Asterinas](https://github.com/asterinas/asterinas/pull/2621)
* [Add the XFCE Nix module](https://github.com/asterinas/asterinas/pull/2670)
* [Add the Podman Nix module](https://github.com/asterinas/asterinas/pull/2683)
* [Enable systemd](https://github.com/asterinas/asterinas/pull/2687)
* [Add Asterinas NixOS ISO installer](https://github.com/asterinas/asterinas/pull/2652)
* [Add Cachix as a source for pre-built binaries](https://github.com/asterinas/asterinas/pull/2685)
* [Add GitHub workflows to publish ISO images](https://github.com/asterinas/asterinas/pull/2743)

## Asterinas Kernel

We have made the following key changes to the Asterinas kernel:

* CPU architectures:
    * x86
        * [Add TSM-based attestation support for TDX via ConfigFS](https://github.com/asterinas/asterinas/pull/2505)
    * RISC-V
        * [Implement arch-aware vDSO](https://github.com/asterinas/asterinas/pull/2319)
        * [Add VirtIO support for RISC-V platforms](https://github.com/asterinas/asterinas/pull/2299)
* Memory management
    * [Support sealing memfd files](https://github.com/asterinas/asterinas/pull/2408)
    * [Support executing memfd files and then `open("/proc/self/exe")`](https://github.com/asterinas/asterinas/pull/2521)
* Process management
    * [Support `CLONE_PARENT` flag](https://github.com/asterinas/asterinas/pull/2447)
    * [Support `execve` in multithreaded process](https://github.com/asterinas/asterinas/pull/2459)
    * [Support `PR_SET`/`GET_SECUREBITS`](https://github.com/asterinas/asterinas/pull/2551)
    * [Fix some `kill`-related behavior](https://github.com/asterinas/asterinas/pull/2516)
* IPC
    * [Add `rt_sigtimedwait` syscall](https://github.com/asterinas/asterinas/pull/2705)
    * [Enqueue ignored signals if the signals are blocked](https://github.com/asterinas/asterinas/pull/2503)
    * [Refactor `NamedPipe` to correct its opening and blocking behaviors](https://github.com/asterinas/asterinas/pull/2434)
    * [Support reopening anonymous pipes from `/proc`](https://github.com/asterinas/asterinas/pull/2694)
    * [Fix or clarify some futexes bugs](https://github.com/asterinas/asterinas/pull/2515)
* File systems
    * VFS
        * [Add `inotify`-related syscalls](https://github.com/asterinas/asterinas/pull/2083)
        * [Add new ioctl infrastructure](https://github.com/asterinas/asterinas/pull/2686)
        * [Add `chmod` and `mkmod` macros](https://github.com/asterinas/asterinas/pull/2440)
        * [Add `syncfs` syscall](https://github.com/asterinas/asterinas/pull/2682)
        * [Add `fchmodat2` syscall](https://github.com/asterinas/asterinas/pull/2666)
        * [Support mount bind with a file](https://github.com/asterinas/asterinas/pull/2418)
        * [Support `MS_REMOUNT` flag](https://github.com/asterinas/asterinas/pull/2432)
        * [Ensure that every `FileLike` is associated with a `dyn Inode`](https://github.com/asterinas/asterinas/pull/2555)
    * Devtmpfs
        * [Add `/dev/full` device](https://github.com/asterinas/asterinas/pull/2439)
        * [Support registering char devices](https://github.com/asterinas/asterinas/pull/2598)
        * [Support registering block device (and their partitions)](https://github.com/asterinas/asterinas/pull/2560)
    * Procfs
        * Add [`/proc/cmdline`](https://github.com/asterinas/asterinas/pull/2420), [`/proc/stat`, `/proc/uptime`](https://github.com/asterinas/asterinas/pull/2370), [`/proc/version`](https://github.com/asterinas/asterinas/pull/2679), [`/proc/[pid]/environ`](https://github.com/asterinas/asterinas/pull/2371), [`/proc/[pid]/oom_score_adj`](https://github.com/asterinas/asterinas/pull/2410), [`/proc/[pid]/cmdline`, `/proc/[pid]/mem`](https://github.com/asterinas/asterinas/pull/2449), [`/proc/[pid]/mountinfo`](https://github.com/asterinas/asterinas/pull/2399), [`/proc/[pid]/fdinfo`](https://github.com/asterinas/asterinas/pull/2526), and [`/proc/[pid]/maps`](https://github.com/asterinas/asterinas/pull/2725)
        * [Support the sleeping and stopping states in `/proc/[pid]/stat`](https://github.com/asterinas/asterinas/pull/2491)
        * [Introduce `VmPrinter`](https://github.com/asterinas/asterinas/pull/2414) and [refactor procfs with `VmPrinter`](https://github.com/asterinas/asterinas/pull/2583)
        * [Fix a lot of bugs in procfs](https://github.com/asterinas/asterinas/pull/2553)
    * Ext2
        * [Support Ext2 handling of FIFO and devices](https://github.com/asterinas/asterinas/pull/2658)
        * [Fix Ext2 directory entry iteration](https://github.com/asterinas/asterinas/pull/2624)
        * [Fix the behavior of syncing BlockGroup metadata in Ext2](https://github.com/asterinas/asterinas/pull/2611)
        * [Fix some bugs in Ext2 superblock](https://github.com/asterinas/asterinas/pull/2675)
    * Configfs
        * [Add basic configfs implementation](https://github.com/asterinas/asterinas/pull/2186)
* Sockets and networking
    * [Support UNIX datagram sockets](https://github.com/asterinas/asterinas/pull/2412)
    * [Add `sendmmsg` syscall](https://github.com/asterinas/asterinas/pull/2676)
    * [Add `sethostname` and `setdomainname` syscalls](https://github.com/asterinas/asterinas/pull/2442)
    * [Support `SO_BROADCAST` and `IP_RECVERR`](https://github.com/asterinas/asterinas/pull/2572)
* Namespace and cgroups
    * [Add the namespace framework (along with `unshare` and `setns` syscalls)](https://github.com/asterinas/asterinas/pull/2312)
    * [Add the mount namespace](https://github.com/asterinas/asterinas/pull/2379)
    * [Support `/proc/[pid]/uid_map` and `/proc/[pid]/gid_map`](https://github.com/asterinas/asterinas/pull/2454)
    * [Implement controller framework for cgroup subsystem](https://github.com/asterinas/asterinas/pull/2282)
    * [Enable process management for cgroups](https://github.com/asterinas/asterinas/pull/2160)
* Devices
    * Input devices
        * [Add the input subsystem](https://github.com/asterinas/asterinas/pull/2364)
        * [Map the I/O memory to the userspace](https://github.com/asterinas/asterinas/pull/2099)
        * [Add the input devices `/dev/input/eventX`](https://github.com/asterinas/asterinas/pull/2561)
        * [Add the framebuffer device `/dev/fb0`](https://github.com/asterinas/asterinas/pull/2216)
        * [Add i8042 mouse](https://github.com/asterinas/asterinas/pull/2479)
        * [Make i8042 initialization stable on real hardware](https://github.com/asterinas/asterinas/pull/2646) 
    * TTY and PTY
        * [Add `KDSETMODE`/`KDSKBMODE` ioctls](https://github.com/asterinas/asterinas/pull/2525)
        * [Fix PTY closing behavior](https://github.com/asterinas/asterinas/pull/2550)
        * [Make PTY master reads block if no PTY slave is open](https://github.com/asterinas/asterinas/pull/2581)
        * [Make the semantics of TTY-related devices correct](https://github.com/asterinas/asterinas/pull/2566)
        * [Support PTY packet mode](https://github.com/asterinas/asterinas/pull/2594)
* System management
    * [Add `reboot` syscall](https://github.com/asterinas/asterinas/pull/2552)
    * [Make `reboot -f` work on real hardware](https://github.com/asterinas/asterinas/pull/2636)
    * [Support `RUSAGE_CHILDREN` for `getrusage`](https://github.com/asterinas/asterinas/pull/2438)
* Misc
    * [Upgrade to Rust 2024 edition and the 20251208 nightly toolchain](https://github.com/asterinas/asterinas/pull/2701)
    * [Introduce the ASCII art of Asterinas logo in gradient colors](https://github.com/asterinas/asterinas/pull/2427)
    * [Add stage support for `init_component` macro](https://github.com/asterinas/asterinas/pull/2415)
    * [Add a new tool called Syscall Compatibility Tracer (`sctrace`)](https://github.com/asterinas/asterinas/pull/2456)

## Asterinas OSTD & OSDK

We have made the following key changes to OSTD and/or OSDK:

* CPU architectures
    * Common
        * [Reorganize `ostd::arch::irq`](https://github.com/asterinas/asterinas/pull/2504)
    * x86
        * [Better x86 CPU feature detection by rewriting all CPUID-related code](https://github.com/asterinas/asterinas/pull/2395)
        * [Set `CR0.WP/NE/MP` explicitly to fix AP behavior](https://github.com/asterinas/asterinas/pull/2422)
        * [Extend cache policies for the x86 Architecture](https://github.com/asterinas/asterinas/pull/2570)
    * RISC-V
        * [Add support for RISC-V PLIC](https://github.com/asterinas/asterinas/pull/2106)
        * [Refactor RISC-V trap handling](https://github.com/asterinas/asterinas/pull/2318)
        * [Add RISC-V FPU support](https://github.com/asterinas/asterinas/pull/2320)
        * [Implement fallible memory operations on RISC-V platform](https://github.com/asterinas/asterinas/pull/2462)
        * [Support bootup on SiFive HiFive Unleashed](https://github.com/asterinas/asterinas/pull/2481)
        * [RISC-V SMP boot](https://github.com/asterinas/asterinas/pull/2368)
        * [Full (<=32) RISC-V SMP support](https://github.com/asterinas/asterinas/pull/2547)
* CPU
    * [Extract `CpuId` into a dedicated sub-module](https://github.com/asterinas/asterinas/pull/2514)
* Memory management
    * [Make `UniqueFrame::repurpose` sound](https://github.com/asterinas/asterinas/pull/2441)
* Interrupt handling
    * [Refactor OSTD's `irq` module for improved clarity](https://github.com/asterinas/asterinas/pull/2429)
* Misc
    * [Move PCI bus out of OSTD](https://github.com/asterinas/asterinas/pull/2027)

## Asterinas Book

We have made the following key changes to the Book:

* [Add the first RFC: "Establish the RFC process"](https://github.com/asterinas/asterinas/pull/2365)
* [Add the second RFC: "Asterinas NixOS"](https://github.com/asterinas/asterinas/pull/2584)
* [Add a new "Limitations on System Calls" section to the book](https://github.com/asterinas/asterinas/pull/2314)
* [Add the Asterinas NixOS volume](https://github.com/asterinas/asterinas/pull/2750)

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

## Asterinas OSTD & OSDK

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
    * [Add support for dynamicall-allocated CPU-local objects](https://github.com/asterinas/asterinas/pull/2036)
    * [Require `T: Send` for `CpuLocal<T, S>`](https://github.com/asterinas/asterinas/pull/2171)
* Memory management:
    * [Adopt a two-phase locking scheme for page tables](https://github.com/asterinas/asterinas/pull/1948)
* Trap handling:
    * [Create `IrqChip` abstraction](https://github.com/asterinas/asterinas/pull/2107)
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
