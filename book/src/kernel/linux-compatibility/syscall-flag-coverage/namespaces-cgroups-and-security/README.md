# Namespaces, Cgroups & Security

<!--
Put system calls such as
unshare, setns, clone (with namespace flags), chroot, pivot_root, prctl,
capset, seccomp, landlock_create_ruleset, landlock_add_rule, 
landlock_restrict_self, and bpf
under this category.
-->

### `prctl`

Supported functionality in SCML:

```c
{{#include prctl.scml}}
```

Partially-supported operations:
* `PR_GET_DUMPABLE` and `PR_SET_DUMPABLE` because coredump is not supported

Unsupported operations:
* `PR_CAP_AMBIENT`, `PR_CAPBSET_READ` and `PR_CAPBSET_DROP`
* `PR_GET_ENDIAN` and `PR_SET_ENDIAN`
* `PR_GET_FP_MODE` and `PR_SET_FP_MODE`
* `PR_GET_FPEMU` and `PR_SET_FPEMU`
* `PR_GET_FPEXC` and `PR_SET_FPEXC`
* `PR_GET_IO_FLUSHER` and `PR_SET_IO_FLUSHER`
* `PR_MCE_KILL` and `PR_MCE_KILL_GET`
* `PR_SET_MM` and `PR_SET_VMA`
* `PR_MPX_ENABLE_MANAGEMENT` and `PR_MPX_DISABLE_MANAGEMENT`
* `PR_PAC_RESET_KEYS`
* `PR_SET_PTRACER`
* `PR_GET_SPECULATION_CTRL` and `PR_SET_SPECULATION_CTRL`
* `PR_SVE_GET_VL` and `PR_SVE_SET_VL`
* `PR_SET_SYSCALL_USER_DISPATCH`
* `PR_GET_TAGGED_ADDR_CTRL` and `PR_SET_TAGGED_ADDR_CTRL`
* `PR_TASK_PERF_EVENTS_ENABLE` and `PR_TASK_PERF_EVENTS_DISABLE`
* `PR_GET_THP_DISABLE` and `PR_SET_THP_DISABLE`
* `PR_GET_TID_ADDRESS`
* `PR_GET_TIMING` and `PR_SET_TIMING`
* `PR_GET_TSC` and `PR_SET_TSC`
* `PR_GET_UNALIGN` and `PR_SET_UNALIGN`
* `PR_GET_AUXV`
* `PR_GET_MDWE` and `PR_SET_MDWE`
* `PR_RISCV_SET_ICACHE_FLUSH_CTX`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/prctl.2.html).

### `seccomp`

Asterinas implements both secure computing modes. `SECCOMP_MODE_STRICT`
restricts a thread to `read`, `write`, `_exit`, and `rt_sigreturn`.
`SECCOMP_MODE_FILTER` runs a thread's chain of classic-BPF filters at the
system-call entry and applies the most restrictive action they return. Filters
may only be installed by a thread that has set `no_new_privs` or that holds
`CAP_SYS_ADMIN`; otherwise installation fails with `EACCES`. Seccomp state
(mode, filters, and `no_new_privs`) is inherited across fork, clone, and execve.

Supported functionality in SCML:

```c
{{#include seccomp.scml}}
```

Supported filter return actions:
* `SECCOMP_RET_ALLOW`
* `SECCOMP_RET_LOG`
* `SECCOMP_RET_ERRNO` (the errno is clamped to `MAX_ERRNO`, i.e. `4095`)
* `SECCOMP_RET_TRAP` (delivers `SIGSYS` with `si_code`, `si_call_addr`,
  `si_syscall`, and `si_arch` filled in)
* `SECCOMP_RET_KILL_THREAD`
* `SECCOMP_RET_KILL_PROCESS`

Recognized but not yet implemented (each safely falls back to returning
`ENOSYS` for the filtered call):
* `SECCOMP_RET_TRACE` (no `ptrace` seccomp tracer)
* `SECCOMP_RET_USER_NOTIF` (no user-space notification via `SECCOMP_IOCTL_NOTIF_*`)

Seccomp filters are read-only classic-BPF programs, so only the instruction
subset that operates on `seccomp_data` and the BPF scratch memory is accepted.
A program is rejected at install time if it uses any other instruction, exceeds
`BPF_MAXINSNS`, jumps out of range, or does not end in a `BPF_RET`.

| Class | Supported | Not supported |
|---|---|---|
| `BPF_LD`/`BPF_LDX` | `W ABS` (loads a `seccomp_data` word), `W IMM`, `W MEM` | `H`/`B` sizes, `LEN`, packet/`IND` addressing |
| `BPF_ST`/`BPF_STX` | scratch-memory store | — |
| `BPF_ALU` | `ADD SUB MUL DIV OR AND LSH RSH MOD XOR NEG` (`K` and `X`) | — |
| `BPF_JMP` | `JA`, `JEQ JGT JGE JSET` (`K` and `X`) | `CALL` |
| `BPF_RET` | `K`, `A` | — |
| `BPF_MISC` | `TAX`, `TXA` | — |

Unsupported flags:
* `SECCOMP_FILTER_FLAG_TSYNC` (synchronizing filters across all threads)
* `SECCOMP_FILTER_FLAG_NEW_LISTENER` and `SECCOMP_FILTER_FLAG_TSYNC_ESRCH`
  (tied to user notification)

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/seccomp.2.html) and
the kernel's `Documentation/userspace-api/seccomp_filter.rst`.

### `capget` and `capset`

Supported functionality in SCML:

```c
{{#include capget_and_capset.scml}}
```

Unsupported versions:
* `_LINUX_CAPABILITY_VERSION_1`
* `_LINUX_CAPABILITY_VERSION_2`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/capget.2.html).

### `unshare`

Supported functionality in SCML:

```c
{{#include unshare.scml}}
```

Unsupported flags:
* `CLONE_NEWCGROUP`
* `CLONE_NEWIPC`
* `CLONE_NEWNET`
* `CLONE_NEWPID`
* `CLONE_NEWTIME`
* `CLONE_NEWUSER`

Silently-ignored flags:
* `CLONE_SYSVSEM`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/unshare.2.html).

### `setns`

Supported functionality in SCML:

```c
{{#include setns.scml}}
```

Unsupported flags:
* `CLONE_NEWCGROUP`
* `CLONE_NEWIPC`
* `CLONE_NEWNET`
* `CLONE_NEWPID`
* `CLONE_NEWTIME`
* `CLONE_NEWUSER`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/setns.2.html).
