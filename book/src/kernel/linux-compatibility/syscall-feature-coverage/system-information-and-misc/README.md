# System Information & Misc.

<!--
Put system calls such as
uname, getrlimit, setrlimit, sysinfo, times, gettimeofday, clock_gettime,
clock_settime, getrusage, getdents, getdents64, personality, syslog,
arch_prctl, set_tid_address, and getrandom
under this category.
-->

### `arch_prctl`

Supported functionality in SCML:

```c
{{#include arch_prctl.scml}}
```

Unsupported codes:
* `ARCH_GET_CPUID` and `ARCH_SET_CPUID`
* `ARCH_GET_GS` and `ARCH_SET_GS`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/arch_prctl.2.html).

### `getrusage`

Supported functionality in SCML:

```c
{{#include getrusage.scml}}
```

Unsupported `who` flags:
* `RUSAGE_CHILDREN`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getrusage.2.html).

### `getrandom`

Supported functionality in SCML:

```c
{{#include getrandom.scml}}
```

Silently-ignored flags:
* `GRND_NONBLOCK` because the underlying operation never blocks

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getrandom.2.html).

## POSIX Clocks

### `clock_gettime`

Supported functionality in SCML:

```c
{{#include clock_gettime.scml}}
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
{{#include clock_nanosleep.scml}}
```

Unsupported clock IDs:
* `CLOCK_TAI`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/clock_nanosleep.2.html).
