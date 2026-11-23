# System Information & Misc.

<!--
Put system calls such as
uname, getrlimit, reboot, setrlimit, sysinfo, times, gettimeofday, clock_gettime,
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

### `reboot`

Supported functionality in SCML:

```c
{{#include reboot.scml}}
```

Unsupported `op` flags:
* `LINUX_REBOOT_CMD_CAD_OFF`
* `LINUX_REBOOT_CMD_CAD_ON`
* `LINUX_REBOOT_CMD_KEXEC`
* `LINUX_REBOOT_CMD_RESTART2`
* `LINUX_REBOOT_CMD_SW_SUSPEND`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/reboot.2.html).

### `sysinfo`

Supported functionality in SCML:

```c
{{#include sysinfo.scml}}
```

Supported returned fields:
* `uptime`, rounded up to seconds when it has a fractional part
* `loads`, containing the 1-, 5-, and 15-minute load averages as fixed-point values scaled by `1 << SI_LOAD_SHIFT`
* `totalram`, `freeram`, `procs`, and `mem_unit`

The `sharedram`, `bufferram`, `totalhigh`, and `freehigh` fields are reported as zero.
The `totalswap` and `freeswap` fields are reported as zero because swap is not supported.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sysinfo.2.html).

### `perf_event_open`

On x86-64, `perf_event_open` (syscall 298) supports per-thread hardware counting
of CPU cycles and retired instructions with a supported Intel or AMD PMU.

Pass `pid = 0` for the caller or a positive thread ID for another thread.
Pass `cpu = -1` to count on whichever CPU runs the thread.
In this mode, the requested event must be supported on every logical CPU.
An explicit CPU number restricts counting to execution on that CPU.
Use `group_fd = -1` for a standalone event.
Each target thread supports one open event of each kind.
Opening a second event of the same kind returns `EBUSY`, even with a different privilege filter.
`/proc/sys/kernel/perf_event_paranoid` controls access to new events.
Writing the setting requires `CAP_SYS_ADMIN` in the initial user namespace.
The default is `3`, which requires `CAP_PERFMON` or `CAP_SYS_ADMIN` for any event.
At level `2`, callers without those capabilities can count user-mode execution.
Levels `1` and below also allow them to count kernel-mode execution.
Cross-process attachment always checks access to the target.
Changes apply to subsequent event opens.

```c
{{#include perf_event_open.scml}}
```

The version 0 attribute layout is supported.
A zero size selects that layout.
Larger layouts up to a page are accepted if their extension bytes are zero.
An unsupported size or nonzero extension returns `E2BIG` and writes the supported size back.
The `disabled` flag leaves the event disabled at creation.
The `enable_on_exec` flag enables the event after the target next successfully executes a program.
The `exclude_user` and `exclude_kernel` flags select which privilege levels to count.
At least one of user and kernel mode must be included.
The `exclude_hv`, `exclude_idle`, and `exclude_guest` flags are accepted for compatibility.
The sample period must be zero.
`PERF_SAMPLE_IDENTIFIER` is accepted as counting metadata.

`read` returns a 64-bit count.
The `read_format` flags append optional fields in the order below.

1. `PERF_FORMAT_TOTAL_TIME_ENABLED` returns the time, in nanoseconds,
   when the event was enabled and the target was scheduled.
2. `PERF_FORMAT_TOTAL_TIME_RUNNING` returns the time, in nanoseconds,
   when the event occupied a running hardware counter.
3. `PERF_FORMAT_ID` returns the event ID.

The event descriptor supports `PERF_EVENT_IOC_ENABLE`, `PERF_EVENT_IOC_DISABLE`,
`PERF_EVENT_IOC_RESET`, and `PERF_EVENT_IOC_ID`.
The first three commands take an argument of zero.

The counting subset supports this invocation of the Linux perf tool.

```sh
perf stat --no-inherit -e cycles:u,instructions:u -- command
```

Sampling, inheritance, event groups, and system-wide counting are currently
unsupported. See [the Linux manual](https://man7.org/linux/man-pages/man2/perf_event_open.2.html).

## POSIX clocks

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
