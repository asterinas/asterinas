# Process & Thread Management

<!--
Put system calls such as
fork, vfork, clone, execve, exit, exit_group, wait4, waitid,
getpid, getppid, gettid, setuid, setgid, getuid, getgid, and prctl
under this category.
-->

## `sched_getattr` and `sched_setattr`

Supported functionality in SCML:

```c
// Get the scheduling policy of a "normal" thread
sched_getattr(
    pid,
    attr = {
        sched_policy = SCHED_OTHER | SCHED_BATCH | SCHED_IDLE,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
// Set the scheduling policy of a "normal" thread
sched_setattr(
    pid,
    attr = {
        sched_policy = SCHED_OTHER | SCHED_BATCH | SCHED_IDLE,
        sched_flags = 0,
        ..
    },
    flags = 0,
);

// Get the scheduling policy of a real-time thread
sched_getattr(
    pid,
    attr = {
        sched_policy = SCHED_FIFO | SCHED_RR,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
// Set the scheduling policy of a real-time thread
sched_setattr(
    pid,
    attr = {
        sched_policy = SCHED_FIFO | SCHED_RR,
        sched_flags = 0,
        ..
    },
    flags = 0,
);
```

Unsupported scheduling policies:
* `SCHED_DEADLINE`

Unsupported scheduling flags:
* `SCHED_FLAG_RESET_ON_FORK`
* `SCHED_FLAG_RECLAIM`
* `SCHED_FLAG_DL_OVERRUN`
* `SCHED_FLAG_UTIL_CLAMP_MIN`
* `SCHED_FLAG_UTIL_CLAMP_MAX`
