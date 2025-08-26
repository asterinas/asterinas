# System Information & Misc.

<!--
Put system calls such as
uname, getrlimit, setrlimit, sysinfo, times, gettimeofday, clock_gettime,
clock_settime, getrusage, getdents, getdents64, personality, syslog,
arch_prctl, set_tid_address, and getrandom
under this category.
-->

## POSIX Clocks

### `clock_gettime`

Supported functionality in SCML:

```c
predefined_clockid = CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW |
                     CLOCK_REALTIME_COARSE | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME |
                     CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID;

// Get the time of a clock specified by a static ID
clock_gettime(clockid = <predefined_clockid>, tp);

// Get the time of a clock specified by a dynamic ID
clock_gettime(clockid = <INTEGER>, tp);
```

Unsupported predefined clock IDs:
* `CLOCK_REALTIME_ALARM`
* `CLOCK_BOOTTIME_ALARM`
* `CLOCK_TAI`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/clock_gettime.2.html).

### `clock_nanosleep`

Supported functionality in SCML:

```c
// Sleep with a specified clock
clock_nanosleep(
    clockid = CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME | CLOCK_PROCESS_CPUTIME_ID,
    flags =
        // Optional flags:
        //
        // Sleep until an absolute time point
        TIMER_ABSTIME,
    t, remain
);
```

Unsupported clock IDs:
* `CLOCK_TAI`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/clock_nanosleep.2.html).

## `arch_prctl`

Supported functionality in SCML:

```c
// Get or set the FS register
arch_prctl(
    code = ARCH_GET_FS | ARCH_SET_FS,
    addr
);
```

Unsupported codes:
* `ARCH_GET_CPUID` and `ARCH_SET_CPUID`
* `ARCH_GET_GS` and `ARCH_SET_GS`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/arch_prctl.2.html).

## `getrusage`

Supported functionality in SCML:

```c
// Return resource usage statistics for the calling process
getrusage(
    who = RUSAGE_SELF,
    usage
);

// Return resource usage statistics for the calling thread
getrusage(
    who = RUSAGE_THREAD,
    usage
);
```

Unsupported `who` flags:
* `RUSAGE_CHILDREN`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getrusage.2.html).

## `getrandom`

Supported functionality in SCML:

```c
// Obtain random bytes
getrandom(
    buf, buflen,
    flags =
        // Optional flags:
        //
        // High-entropy pool
        GRND_RANDOM
);
```

Silently-ignored flags:
* `GRND_NONBLOCK` because the underlying operation never blocks

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getrandom.2.html).