# Process & Thread Management

<!--
Put system calls such as
fork, vfork, clone, execve, exit, exit_group, wait4, waitid,
getpid, getppid, gettid, setuid, setgid, getuid, getgid, and prctl
under this category.
-->

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