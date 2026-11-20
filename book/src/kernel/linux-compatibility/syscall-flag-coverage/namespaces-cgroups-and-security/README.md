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

Supported functionality in SCML:

```c
{{#include seccomp.scml}}
```

`SECCOMP_SET_MODE_STRICT` requires `flags = 0` and a null argument.
`prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT, ...)` ignores the filter argument.
Strict and filter modes are mutually exclusive.
Strict mode permits `read`, `write`, `exit`, and `rt_sigreturn`.
On x86-64, the allowlist also admits `uretprobe` and `uprobe`,
which currently return `ENOSYS`.
A syscall outside the allowlist terminates the calling thread with `SIGKILL`.
On x86-64, executing `RDTSC` or `RDTSCP` in user mode under strict mode
raises `SIGSEGV`.

`SECCOMP_GET_ACTION_AVAIL` and `SECCOMP_GET_NOTIF_SIZES` also require `flags = 0`.
`SECCOMP_GET_ACTION_AVAIL` accepts `SECCOMP_RET_USER_NOTIF` and `SECCOMP_RET_LOG`.

By default, `SECCOMP_SET_MODE_FILTER` installs a filter on the calling thread
and returns `0` on success.
`SECCOMP_FILTER_FLAG_TSYNC` shares the resulting filter chain with the other
live threads and propagates the caller's `no_new_privs` setting when it is set.
A target is eligible if seccomp is disabled on it or its filter chain is the
same as the caller's current chain or an ancestor of it. Independently installed
filters form different chains even if their programs are identical.
If a thread is incompatible, the kernel rejects the installation and returns
the first incompatible thread's TID, or fails with `ESRCH` if
`SECCOMP_FILTER_FLAG_TSYNC_ESRCH` is set.
`SECCOMP_FILTER_FLAG_TSYNC_ESRCH` has no effect without `SECCOMP_FILTER_FLAG_TSYNC`.

`SECCOMP_FILTER_FLAG_NEW_LISTENER` returns a close-on-exec listener file descriptor on success.
Combining it with `SECCOMP_FILTER_FLAG_TSYNC` requires `SECCOMP_FILTER_FLAG_TSYNC_ESRCH`.
When the winning filter returns `SECCOMP_RET_USER_NOTIF` and has an open listener,
the calling thread waits for the supervisor to respond. With no open listener,
the syscall returns `ENOSYS`.
The listener supports `SECCOMP_IOCTL_NOTIF_RECV` to receive a request,
`SECCOMP_IOCTL_NOTIF_SEND` to respond or continue the syscall,
`SECCOMP_IOCTL_NOTIF_ID_VALID` to check a request ID, and
`SECCOMP_IOCTL_NOTIF_ADDFD` to install a descriptor in the target thread's table.
See [Seccomp listener commands](../file-descriptor-and-io-control/README.md#seccomp-listener-ioctls)
for ioctl argument requirements.
Polling the listener reports `POLLHUP` when the last thread using its filter exits.
`SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV` requires `SECCOMP_FILTER_FLAG_NEW_LISTENER`
and makes the target's notification wait killable after the listener receives the notification.
`SECCOMP_FILTER_FLAG_SPEC_ALLOW` is accepted and has no effect on supported architectures.

When `SECCOMP_RET_LOG` wins, the kernel logs the syscall and lets it proceed.
`SECCOMP_FILTER_FLAG_LOG` requests logging for eligible actions from the winning filter.
`SECCOMP_RET_ALLOW` produces no record, and a `TRACE` or `USER_NOTIF` decision
is logged only if it causes syscall execution to be skipped.
Asterinas sends seccomp records to the kernel log console independently of `loglevel`.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/seccomp.2.html).

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
* `CLONE_NEWNET`
* `CLONE_NEWPID`
* `CLONE_NEWTIME`
* `CLONE_NEWUSER`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/setns.2.html).
