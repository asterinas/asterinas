# Signals & Timers

<!--
Put system calls such as

rt_sigaction, rt_sigprocmask, rt_sigpending, rt_sigqueueinfo, rt_tgsigqueueinfo,
rt_sigreturn, kill, tkill, tgkill, alarm, setitimer, getitimer, nanosleep,
timer_create, timer_settime, timer_gettime, and timer_delete
under this category.
-->

## Signals

### `rt_sigaction`

Supported functionality in SCML:

```c
// Change and/or retrieve a signal action
rt_sigaction(
    signum,
    act = {
        sa_flags = SA_ONSTACK | SA_RESTART | SA_NODEFER | SA_RESTORER | SA_SIGINFO | SA_RESETHAND,
        ..
    },
    oldact, sigsetsize
);
```

Unsupported `sigaction` flags:
* `SA_NOCLDSTOP`
* `SA_NOCLDWAIT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sigaction.2.html).

### `rt_sigprocmask`

Supported functionality in SCML:

```c
// Change and/or retrieve blocked signals
rt_sigprocmask(
    how = SIG_BLOCK | SIG_UNBLOCK | SIG_SETMASK, set, oldset, sigsetsize
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sigprocmask.2.html).

## POSIX Interval Timers

### `timer_create`

Supported functionality in SCML:

```c
opt_notify_methods = SIGEV_NONE | SIGEV_SIGNAL | SIGEV_THREAD_ID;

// Create a timer with predefined clock source
timer_create(
    clockid = CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID | CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME,
    sevp = <opt_notify_methods>,
    timerid
);

// Create a timer based on a per-process or per-thread clock
timer_create(
    clockid = <INTEGER>,
    sevp = <opt_notify_methods>,
    timerid
);
```

Unsupported predefined clock IDs:
* `CLOCK_REALTIME_ALARM`
* `CLOCK_BOOTTIME_ALARM`
* `CLOCK_TAI`

Unsupported notification methods:
* `SIGEV_THREAD`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/timer_create.2.html).
