# Hardware counting with `perf stat`

Run the matrix in an x86-64 Asterinas guest with an exposed Intel or AMD PMU,
a stock Linux `perf` binary in `PATH`, and permission to open perf events.

The regression build packages the runner and its single-threaded workload together.
The workload runs a user-space loop and issues a system call on each outer iteration.

```sh
cd /test/io/perf_event_open
sh ./perf_stat_matrix.sh ./stat_workload /tmp/perf-stat-matrix
```

Use a new results directory for each run. The matrix runs each of the 15
nonempty combinations ten times. The two events are independent, and each
can count user space, kernel space, or both. Each invocation must exit
successfully and return exactly one positive count with positive running time
and a valid running percentage for each requested event.
The workload writes a completion marker only after its final iteration.
The matrix requires that marker even when `perf stat` reports success.
It also compares the median retired-instruction counts for the single-event runs.
The combined scope must exceed either individual scope
and be within 15% of their sum.

`cases.csv` records the event selection for each case and `counts.csv` holds
the 240 individual event measurements. The directory also contains the raw
`perf stat` CSV for each invocation and the kernel and perf version strings.

## Third-party counting tests

Build `instructions`, `cycles`, and `ioctl_7_id` from Vince Weaver's
[`perf_event_tests` at `a0d5156c`](https://github.com/deater/perf_event_tests/tree/a0d5156c81f348107329955c4490dc362de77ddb)
in an x86-64 Linux build environment.

```sh
make -C lib
make -C tests/generalized_events LDFLAGS=-static instructions cycles
make -C tests/ioctl LDFLAGS=-static ioctl_7_id
```

Copy the three binaries to a directory in the x86-64 guest.
From `/test/io/perf_event_open`, run:

```sh
sh ./run_vince_perf_event_tests.sh /path/to/upstream-binaries
```

The runner requires all three upstream tests to report a pass.
They check counts for known instruction sequences, user-mode cycles during
sleep and computation, and the event ID returned by `ioctl` and `read`.

## Capability changes

Run `/test/io/perf_event_open/capset_revoke` from the root shell of a PMU-enabled x86-64 guest.
The regression build includes this executable.
It checks that unchanged or reduced capabilities preserve counting
and that gaining an effective capability freezes the existing event.

## Passed validation cases

The following cases passed in an x86-64 AMD KVM guest with two vCPUs.

| Test | Result |
| --- | --- |
| [LTP `perf_event_open01`](https://github.com/linux-test-project/ltp/blob/20260529/testcases/kernel/syscalls/perf_event_open/perf_event_open01.c) (20260529) | The hardware instruction-counting subtest reported `TPASS`. |
| Vince Weaver's `perf_event_tests` (`a0d5156c`) | `instructions`, `cycles`, and `ioctl_7_id` passed. |
| Stock Linux `perf stat` (7.1.4) | All 15 combinations passed in all 10 rounds, yielding 240 valid event counts. |
