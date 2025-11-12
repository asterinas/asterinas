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
* `PR_GET_NO_NEW_PRIVS` and `PR_SET_NO_NEW_PRIVS`
* `PR_PAC_RESET_KEYS`
* `PR_SET_PTRACER`
* `PR_GET_SECCOMP` and `PR_SET_SECCOMP`
* `PR_GET_SECUREBITS` and `PR_SET_SECUREBITS`
* `PR_GET_SPECULATION_CTRL` and `PR_SET_SPECULATION_CTRL`
* `PR_SVE_GET_VL` and `PR_SVE_SET_VL`
* `PR_SET_SYSCALL_USER_DISPATCH`
* `PR_GET_TAGGED_ADDR_CTRL` and `PR_SET_TAGGED_ADDR_CTRL`
* `PR_TASK_PERF_EVENTS_ENABLE` and `PR_TASK_PERF_EVENTS_DISABLE`
* `PR_GET_THP_DISABLE` and `PR_SET_THP_DISABLE`
* `PR_GET_TID_ADDRESS`
* `PR_GET_TIMERSLACK` and `PR_SET_TIMERSLACK`
* `PR_GET_TIMING` and `PR_SET_TIMING`
* `PR_GET_TSC` and `PR_SET_TSC`
* `PR_GET_UNALIGN` and `PR_SET_UNALIGN`
* `PR_GET_AUXV`
* `PR_GET_MDWE` and `PR_SET_MDWE`
* `PR_RISCV_SET_ICACHE_FLUSH_CTX`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/prctl.2.html).

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
