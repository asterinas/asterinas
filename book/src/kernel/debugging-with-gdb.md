# Debugging the Kernel with GDB

## Overview

Asterinas provides GDB helper scripts in `scripts/gdb/`.
The helpers can be loaded in `rust-gdb` to inspect kernel objects,
format common wrapper types, and run inspection commands.

The helpers are written in Python and are organized into several modules.

| Module | Description |
|--------|-------------|
| `helper/layout.py` | Reads generic Rust wrappers, collections, atomics, and locks |
| `helper/kernel.py` | Traverses Asterinas kernel objects |
| `helper/printers.py` | Registers pretty-printers for wrapper types |
| `helper/commands.py` | Implements `ast-*` inspection commands |
| `helper/gdb_bridge.py` | Contains small wrappers around the GDB Python API |

## Loading the helpers

When `cargo osdk debug` is run from the Asterinas workspace,
it loads the helpers automatically.

The helpers can also be loaded manually after connecting `rust-gdb`
to a running kernel.

```gdb
(gdb) source scripts/gdb/asterinas-gdb.py
```

Plain `gdb` can source the script, but some helper functions depend on
the Rust pretty-printers installed by `rust-gdb`.

## Setting breakpoints

The QEMU GDB server may stop at the x86 reset vector before the kernel
is mapped at its virtual address. In this state, normal software
breakpoints cannot be inserted into kernel text.

Use hardware breakpoints when stopping in early kernel code.

```gdb
(gdb) hbreak __ostd_main
(gdb) continue
```

After the kernel has started, continue to a point where the objects of
interest already exist. For example, the first syscall is late enough
for PID 1 to be present in the PID table.

```gdb
(gdb) hbreak aster_kernel::syscall::handle_syscall
(gdb) continue
```

## Pretty-printers

The helpers register pretty-printers for the wrapper types that commonly
hide useful values during debugging.

| Printer | Types |
|---------|-------|
| `AtomicScalar` | `core::sync::atomic::Atomic<T>` |
| `OstdMutex` | `ostd::sync::Mutex<T>` |
| `OstdRwMutex` | `ostd::sync::RwMutex<T>` |
| `OstdSpinLock` | `ostd::sync::SpinLock<T, G>` |

Atomic values are printed as scalars.
Lock wrappers show the lock state and expose the wrapped value as a child.

Examples:

```gdb
(gdb) p (*$ast_thread(1)).is_exited
(gdb) p aster_kernel::process::pid_table::PID_TABLE
```

Use `/r` to print the raw DWARF layout when the wrapper internals are
needed.

```gdb
(gdb) p/r (*$ast_thread(1)).is_exited
```

## Convenience functions

Convenience functions return pointer-like `gdb.Value` objects.
They are useful when the desired object is reachable only through
several kernel data structures.

| Function | Returns |
|----------|---------|
| `$ast_process(pid)` | `Process *` for the given PID |
| `$ast_thread(tid)` | `Thread *` for the given TID |
| `$ast_pid_table()` | `PidTable *` for the global PID table |
| `$ast_file_table(pid)` | `FileTable *` for the given process |

Examples:

```gdb
(gdb) p *$ast_process(1)
(gdb) p (*$ast_process(1)).pid
(gdb) p *$ast_thread(1)
(gdb) p *$ast_file_table(1)
```

## Commands

The helpers also provide commands for inspection tasks that require
iteration or formatted output.

| Command | Description |
|---------|-------------|
| `ast-version` | Print the kernel version |
| `ast-ps [PID]` | List processes, optionally filtered by PID |
| `ast-threads` | List threads and their owning PIDs |
| `ast-pstree` | Show the process tree |
| `ast-fds <PID>` | List file descriptors for a process |
| `ast-uptime` | Print uptime from kernel jiffies |

Example:

```gdb
(gdb) ast-ps
   PID    PPID  STATE      THREADS  NAME
     1       0  Running          1  init
```

## Smoke test

The smoke test boots the kernel with a GDB server, attaches `rust-gdb`,
loads the helpers, and checks printers, convenience functions, and
commands.

```bash
make gdb-smoke-test
```

The same test is run by `.github/workflows/test_gdb_helpers.yml`.

## Maintenance

The helpers depend on Rust symbol names, type paths, and struct layouts.
Hardcoded names and constants are kept in
`scripts/gdb/helper/constants.py`.
Generic Rust layout traversal is implemented in
`scripts/gdb/helper/layout.py`.
Asterinas-specific traversal is implemented in
`scripts/gdb/helper/kernel.py`.

Rust definitions that are used by the helpers carry a `COUPLED` marker.
When changing a marked definition, update the referenced helper code in
the same patch.

```bash
grep -rn "// COUPLED: scripts/gdb" kernel/ ostd/
```
