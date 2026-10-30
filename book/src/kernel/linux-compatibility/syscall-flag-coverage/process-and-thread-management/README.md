# Process & Thread Management

<!--
Put system calls such as
fork, vfork, clone, execve, exit, exit_group, wait4, waitid,
getpid, getppid, gettid, setuid, setgid, getuid, getgid, prctl and ptrace
under this category.
-->

### `execve` and `execveat`

Both calls support the arguments and flags listed in `fully_covered.scml`.
When AppArmor is selected at boot,
an unconfined task can acquire a profile through an exact-path attachment during exec.
The attachment path identifies the opened original executable,
including the script for an interpreter-based execution,
relative to the task's mount namespace root.
Symlinks use their resolved target;
hardlinks retain their distinct paths.
Deleted, anonymous, and unreachable executable paths do not acquire an attachment.

AppArmor support currently covers label binding and observation only:

- Loading a profile does not relabel tasks already running.
- Fork/clone preserve the label through shared immutable state and independent credential slots.
- A labelled task preserves its label across subsequent exec calls.
- General pattern attachments, profile transitions, stacking, replacement, and deletion are unsupported.
- `enforce` and `complain` describe the loaded profile mode;
  profile permissions are not enforced yet.

The read-only `/proc/<pid>/attr/current` attribute reports the main thread's label.
`/proc/<pid>/task/<tid>/attr/current` reports the specified thread's label.
The result is `unconfined\n` or `<profile> (<mode>)\n`.
Opening checks access using the caller's filesystem credentials and captures an immutable snapshot.
An open descriptor retains that snapshot across exec, exit, and partial reads.
A new open observes the then-current label.
Denied access returns `EACCES`.
A missing PID or TID during path lookup returns `ENOENT`;
a target that becomes unavailable before the attribute is opened returns `ESRCH`.
No selected provider returns `EINVAL`.
Writes cannot switch labels and are rejected.

For boot selection and defaults,
see [Kernel Parameters](../../kernel-parameters.md).

### `sched_getattr` and `sched_setattr`

Supported functionality in SCML:

```c
{{#include sched_getattr_and_sched_setattr.scml}}
```

Unsupported scheduling policies:
* `SCHED_DEADLINE`

Unsupported scheduling flags:
* `SCHED_FLAG_RESET_ON_FORK`
* `SCHED_FLAG_RECLAIM`
* `SCHED_FLAG_DL_OVERRUN`
* `SCHED_FLAG_UTIL_CLAMP_MIN`
* `SCHED_FLAG_UTIL_CLAMP_MAX`

### `wait4`

Supported functionality in SCML:

```c
{{#include wait4.scml}}
```

Ignored options:
* `WEXITED`
* `WNOTHREAD`
* `WALL`
* `WCLONE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/wait4.2.html).

### `clone` and `clone3`

Supported functionality in SCML:

```c
{{#include clone_and_clone3.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/clone.2.html).

### `sched_setscheduler`

Supported functionality in SCML:

```c
{{#include sched_setscheduler.scml}}
```

Unsupported policies or flags:
* `SCHED_RESET_ON_FORK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sched_setscheduler.2.html).

### `waitid`

Supported functionality in SCML:

```c
{{#include waitid.scml}}
```

Ignored options:
* `WEXITED`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/waitid.2.html).

### `ptrace`

Supported functionality in SCML:

```c
{{#include ptrace.scml}}
```

Supported requests:
* `PTRACE_TRACEME`
* `PTRACE_PEEKTEXT`
* `PTRACE_PEEKDATA`
* `PTRACE_PEEKUSER` (x86-64 only)
* `PTRACE_POKETEXT`
* `PTRACE_POKEDATA`
* `PTRACE_POKEUSER` (x86-64 only)
* `PTRACE_CONT`
* `PTRACE_KILL`
* `PTRACE_SINGLESTEP` (x86-64 only)
* `PTRACE_GETREGS` (x86-64 only)
* `PTRACE_SETREGS` (x86-64 only)
* `PTRACE_SYSCALL`
* `PTRACE_SETOPTIONS`
* `PTRACE_GETEVENTMSG`
* `PTRACE_GETSIGINFO`

Limitations:
* Only the main thread of a process can act as the tracer.
* `PTRACE_PEEKUSER` and `PTRACE_POKEUSER` only support offsets for general-purpose registers.
* If a tracee has clone-family options (`PTRACE_O_TRACEFORK`, `PTRACE_O_TRACEVFORK`, `PTRACE_O_TRACEVFORKDONE`, or `PTRACE_O_TRACECLONE`) and then performs a clone-family operation that would trigger a ptrace event, the clone-family operation returns `EOPNOTSUPP`.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ptrace.2.html).
