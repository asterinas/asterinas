# Syscall Compatibility Tracer

Syscall Compatibility Tracer (`sctrace`) is an oracle
to answer the following questions
for developers and users of a Linux ABI-compatible OS
(such as [Asterinas](https://github.com/asterinas/asterinas)):
**Is a target Linux application supported by the OS?
If not, where are the gaps?**

## Motivation

There are tons of Linux ABI-compatible OSes out there:
some of them have been deployed at scale
(e.g., [HongMeng Kernel](https://www.usenix.org/conference/osdi24/presentation/chen-haibo) and [gVisor](https://gvisor.dev/)),
some are in rapid development
(e.g., [Asterinas](https://github.com/asterinas/asterinas)),
some target niche markets
(e.g., [Occlum](https://github.com/occlum/occlum)),
some are research prototypes
(e.g., [Graphene](https://grapheneproject.io/)),
and some are just hobby projects
(e.g., [Maestro](https://github.com/maestro-os/maestro)).
They all provide a subset of Linux system calls and features
so that at least some Linux applications can run on them _unmodified_.

But here is a pain point for the developers and early adopters
of such a Linux ABI-compatible OS:
**when a Linux application is to be ported to this (imperfectly)
Linux ABI-compatible OS,
how can we know beforehand if the target application
is supposed to be supported or not?
And if not, where are the gaps?**

A common practice is to run the target Linux application
with the classic [strace](https://strace.io/) tool,
which traces and prints all system calls invoked by an application.
For example, running a Hello World program with `strace`:

```bash
strace ./hello_world
```

would generate output as shown below:

```
execve("./hello_world", ["./hello_world"], 0xffffffd3f710 /* 4 vars */) = 0
brk(NULL)                                = 0xaaaabdc1b000
mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0xffff890f4000
openat(AT_FDCWD, "/lib/aarch64-linux-gnu/libc.so.6", O_RDONLY|O_CLOEXEC) = 3
read(3, "\177ELF\2\1\1\3\0\0\0\0\0\0\0\0\3\0\267\0\1\0\0\0\360\206\2\0\0\0\0\0"..., 832) = 832
fstat(3, {st_mode=S_IFREG|0755, st_size=1722920, ...}) = 0
...
write(1, "Hello, World!\n", 14)          = 14
exit_group(0)                            = ?
```

As `strace` captures all interactions between the application
and the OS kernel,
its log provides sufficient information to assess if there are any
compatibility issues.
But `strace`-ing a complex application might output millions of lines of log.
It would be too tedious for a human to review.
Writing an ad-hoc log processing script or tool would greatly reduce the human labor.
But this approach is error-prone and its results would be inaccurate
as it lacks the ground truth about all supported (or unsupported)
Linux system calls of the target OS.

This is where `sctrace` can be a life saver.

## Introduction

We introduce Syscall Compatibility Tracer (`sctrace`),
a tool that checks whether all system calls invoked
by a target Linux application are supported
by a target Linux ABI-compatible OS or not.
To achieve this goal,
it combines the classic `strace` tool
with a mini domain-specific language called
[System Call Matching Language (SCML)](https://asterinas.github.io/book/kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.html).
SCML adopts a `strace`-inspired syntax,
with which one can specify all supported system call patterns
in a concise, accurate, and human-readable way.

The `sctrace` tool originates from the [Asterinas](https://github.com/asterinas/asterinas) project
and is released so that it may be useful to the wider OS community.

## Getting Started

### Installation

The `sctrace` tool has two prerequisites:

* [**strace**](https://strace.io/) version 5.15 or higher
    * Install on Debian/Ubuntu: `sudo apt install strace`
    * Install on Fedora/RHEL: `sudo dnf install strace`
* The Rust toolchain (the version `nightly-2025-12-06` is tested; other versions may be supported as well)
    * Install via [Rustup](https://rustup.rs/): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

To install the `sctrace` tool, execute the following command:

```bash
cargo install sctrace
```

### Basic syntax

```bash
sctrace <scml_file>... -- <prog> <arg>...
```

* `<scml_file>...` gives one or more SCML files
that specify the supported system call patterns of a target OS.
* `<prog>` gives the name or path of the target Linux program.
* `<arg>...` provides zero, one, or more arguments for `<prog>`.

### A Hello World Example

As `sctrace` requires SCML files as its input,
let's write a simple one called `open.scml`:

```c
// Some (not all) valid flag patterns for the `open` and `openat` syscalls
open_flags = O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC;

// Some (not all) valid patterns for the `open` syscall
open(path, flags = <open_flags>);
// Some (not all) valid patterns for the `openat` syscall
openat(dirfd, path, flags = <open_flags>);
```

This file describes some (not all) valid patterns
for Linux's [`open` and `openat`](https://man7.org/linux/man-pages/man2/open.2.html) system calls.
SCML's syntax and semantics resemble those of `strace` and the C language.
For more explanation about SCML syntax,
see its [documentation](https://asterinas.github.io/book/kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.html).

To see `sctrace` in action,
we now use it to track the execution of a simple command
that prints out the content of `open.scml`:

```bash
sctrace open.scml -- cat open.scml
```

The output would look like below:

```
1045884 execve("/usr/bin/cat", ["cat", "open.scml"], 0x5d08ad413588 /* 25 vars */)           = 0 (unsupported)
1045884 brk(NULL)                       = 0x5a3f59f86000 (unsupported)
1045884 mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0x7d321cf1f000 (unsupported)
1045884 access("/etc/ld.so.preload", R_OK) = -1 ENOENT (No such file or directory) (unsupported)
1045884 openat(AT_FDCWD</home/sutao>, "/etc/ld.so.cache", O_RDONLY|O_CLOEXEC) = 5</etc/ld.so.cache>
1045884 fstat(5</etc/ld.so.cache>, {st_mode=S_IFREG|0644, st_size=39847, ...}) = 0 (unsupported)
1045884 mmap(NULL, 39847, PROT_READ, MAP_PRIVATE, 5</etc/ld.so.cache>, 0) = 0x7d321cf15000 (unsupported)
1045884 close(5</etc/ld.so.cache>)      = 0 (unsupported)
1045884 openat(AT_FDCWD</home/sutao>, "/lib/x86_64-linux-gnu/libc.so.6", O_RDONLY|O_CLOEXEC) = 5</usr/lib/x86_64-linux-gnu/libc.so.6>
1045884 read(5</usr/lib/x86_64-linux-gnu/libc.so.6>, "\177ELF\2\1\1\3\0\0\0\0\0\0\0\0\3\0>\0\1\0\0\0\220\243\2\0\0\0\0\0"..., 832) = 832 (unsupported)
```

The `(unsupported)` tag is appended to almost every system call entry
(except `open` and `openat`)
as `sctrace` only recognizes the valid patterns specified by `open.scml`.
Expanding the pattern rules in the input SCML files would allow `sctrace`
to recognize more system calls, as we will show later.

## User Guide

### Command-Line Interface (CLI)

The `sctrace` tool supports two modes:
the *online* and *offline* modes.

#### Online Mode (Real-time Tracking)

In the online mode, `sctrace` tracks a running command in real time.
This is the mode we described in the Hello World example.
Its complete CLI syntax is shown below:

```bash
sctrace <scml_file>... [--quiet] -- <prog> <arg>...
```

If the `--quiet` option is given,
then only the **unsupported** system calls are shown in the output,
making it easier to spot compatibility gaps.

#### Offline Mode (Log Analysis)

The offline mode does not run a user-given command;
instead, it analyzes a user-given `strace` log of a command.
The CLI syntax is shown below:

```bash
sctrace <scml_file>... [--quiet] --input <strace_log>
```

The input file `strace_log` is expected to be generated
using the following specific form of `strace`:

```bash
strace -yy -f -o <strace_log> <prog> <arg>...
```

The meaning of the `--quiet` option is the same as that in the online mode.

### Using `sctrace` for Asterinas

The syscall coverage of Asterinas has been formally [documented](https://asterinas.github.io/book/kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.html) in SCML.
The entire set of SCML files can be found
in the [`syscall-flag-coverage/`](../../book/src/kernel/linux-compatibility/syscall-flag-coverage/) directory of the Asterinas book.

To fetch these SCML files, run the following command:

```bash
git clone --depth 1 https://github.com/asterinas/asterinas
cd asterinas/book/src/kernel/linux-compatibility/syscall-flag-coverage/
```

You can now leverage these files to
check if a program can be ported to Asterinas:

```bash
sctrace $(find . -name "*.scml") -- <prog> <arg>...
```

In the Asterinas development Docker image,
we have pre-installed the `sctrace` tool.
For convenience,
the Docker image sets an environment variable called `ASTER_SCML`,
which is the list of all Asterinas SCML files.
This helps simplify using `sctrace` for Asterinas.

```bash
sctrace $ASTER_SCML [--quiet] -- <prog> <arg>...
sctrace $ASTER_SCML [--quiet] --input <strace_log>
```

### Troubleshooting

For online tracing, you may need elevated privileges
to attach to the target process using `ptrace`:

```bash
sudo sctrace patterns.scml -- target_program
```

## Developer Guide

The source code of `sctrace` resides within the Asterinas project.
So the first step is to download the Asterinas codebase:

```bash
git clone https://github.com/asterinas/asterinas
```

The `sctrace` tool can be located in `tools/sctrace/`:

```bash
cd tools/sctrace
```

The tool is written in Rust.
So you will need to use Cargo to build and test it.

```bash
cargo build
cargo test
```
