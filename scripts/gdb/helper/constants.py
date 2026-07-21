# SPDX-License-Identifier: MPL-2.0

"""
Single source of truth for kernel-side knowledge the GDB helpers depend on.

When any entry here becomes wrong (renamed symbol, changed layout),
the helpers may fail or produce misleading output.  Any developer
updating a coupled Rust data structure should grep this file.

Each entry cross-references the Rust source file that must stay in sync.
Rust-side ``COUPLED`` markers point at the helper module that traverses
the coupled layout.
"""

# --- Crate prefixes ---
_CRATE = "aster_kernel"
_OSTD = "ostd"
_ALLOC = "alloc"

# --- Global symbols ---

# COUPLED -> kernel/src/process/pid_table.rs
PID_TABLE_SYMBOL = f"{_CRATE}::process::pid_table::PID_TABLE"

# COUPLED -> ostd/src/timer/jiffies.rs
ELAPSED_JIFFIES_SYMBOL = f"{_OSTD}::timer::jiffies::ELAPSED"

# COUPLED -> ostd/src/timer/mod.rs
TIMER_FREQ_HZ = 1000

# --- Box<dyn Any> downcast type names ---

# COUPLED -> kernel/src/thread/mod.rs
TASK_DATA_CONCRETE = (
    f"{_ALLOC}::sync::Arc<{_CRATE}::thread::Thread, "
    f"{_ALLOC}::alloc::Global>"
)

# COUPLED -> kernel/src/process/posix_thread/mod.rs
THREAD_DATA_CONCRETE = f"{_CRATE}::process::posix_thread::PosixThread"

# --- Layout constants ---

# COUPLED -> kernel/src/process/posix_thread/name.rs
THREAD_NAME_MAX_LEN = 16

# --- File vtable markers ---
# Used to identify Arc<dyn FileLike> concrete type from its vtable symbol.
# COUPLED -> kernel/src/fs/file/file_table.rs
FILE_VTABLE_MARKERS = (
    "InodeFile",
    "Socket",
    "EventFile",
    "EpollFile",
    "TimerFile",
    "PipeReader",
    "PipeWriter",
    "Pipe",
    "UnixStream",
    "TcpSocket",
    "UdpSocket",
    "DevPts",
)
