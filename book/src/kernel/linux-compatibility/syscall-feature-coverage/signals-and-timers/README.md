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
{{#include rt_sigaction.scml}}
```

Unsupported `sigaction` flags:
* `SA_NOCLDSTOP`
* `SA_NOCLDWAIT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sigaction.2.html).

### `rt_sigprocmask`

Supported functionality in SCML:

```c
{{#include rt_sigprocmask.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sigprocmask.2.html).

## POSIX Interval Timers

### `timer_create`

Supported functionality in SCML:

```c
{{#include timer_create.scml}}
```

Unsupported predefined clock IDs:
* `CLOCK_REALTIME_ALARM`
* `CLOCK_BOOTTIME_ALARM`
* `CLOCK_TAI`

Unsupported notification methods:
* `SIGEV_THREAD`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/timer_create.2.html).
