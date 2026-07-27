# SPDX-License-Identifier: MPL-2.0

"""
Asterinas kernel object navigation.

This module is the deep interface between the Rust kernel layout and the
GDB-facing commands.  It returns kernel values, rows, and snapshots while
keeping field paths such as ``Process.tasks`` and ``PosixThread.name`` local.
"""

from dataclasses import dataclass

import gdb

from helper import gdb_bridge
from helper.constants import (
    ELAPSED_JIFFIES_SYMBOL,
    FILE_VTABLE_MARKERS,
    PID_TABLE_SYMBOL,
    TASK_DATA_CONCRETE,
    THREAD_DATA_CONCRETE,
    THREAD_NAME_MAX_LEN,
    TIMER_FREQ_HZ,
)
from helper.layout import (
    IntrospectionError,
    lookup_type,
    read_btree_map,
    read_global,
    read_scalar,
    read_slot_vec,
    read_vec,
    unwrap_arc,
    unwrap_mutex,
    unwrap_option,
    unwrap_weak,
)


@dataclass(frozen=True)
class ProcessRow:
    pid: int
    ppid: int
    state: str
    thread_count: int
    name: str


@dataclass(frozen=True)
class ThreadRow:
    tid: int
    pid: int
    name: str


@dataclass(frozen=True)
class FdRow:
    fd: int
    flags: int
    fd_type: str
    flag_text: str


@dataclass(frozen=True)
class UptimeSnapshot:
    jiffies: int
    frequency_hz: int


# --- PID table navigation ---
#
# COUPLED: kernel/src/process/pid_table.rs
#   PID_TABLE: Mutex<PidTable>
#   PidTable { entries: BTreeMap<u32, Arc<PidEntry>>, ... }
#   PidEntry { inner: Mutex<PidEntryInner> }
#   PidEntryInner { thread: Weak<Thread>, process: Weak<Process>, ... }


def get_pid_table():
    """Return the global ``PidTable`` inner value."""
    table_mutex = read_global(PID_TABLE_SYMBOL)
    if table_mutex is None:
        raise IntrospectionError(f"Cannot resolve symbol {PID_TABLE_SYMBOL}")
    _, pid_table = unwrap_mutex(table_mutex)
    return pid_table


def _unwrap_pid_entry(arc_entry):
    entry = unwrap_arc(arc_entry)
    _, inner = unwrap_mutex(entry['inner'])
    return inner


def iter_processes():
    """Yield ``(pid, Process)`` for every live process."""
    entries = get_pid_table()['entries']
    for key, arc_entry in read_btree_map(entries):
        entry = _unwrap_pid_entry(arc_entry)
        process = unwrap_weak(entry['process'])
        if process is not None:
            yield (int(key), process)


def iter_threads():
    """Yield ``(tid, Thread)`` for every live thread."""
    entries = get_pid_table()['entries']
    for key, arc_entry in read_btree_map(entries):
        entry = _unwrap_pid_entry(arc_entry)
        thread = unwrap_weak(entry['thread'])
        if thread is not None:
            yield (int(key), thread)


def find_process(pid):
    """Return the ``Process`` for ``pid`` or ``None``."""
    for entry_pid, process in iter_processes():
        if entry_pid == int(pid):
            return process
    return None


def find_thread(tid):
    """Return the ``Thread`` for ``tid`` or ``None``."""
    for entry_tid, thread in iter_threads():
        if entry_tid == int(tid):
            return thread
    return None


# --- Task -> Thread -> PosixThread navigation ---
#
# COUPLED: kernel/src/thread/mod.rs, kernel/src/process/posix_thread/mod.rs
#   Task.data: Box<dyn Any + Send + Sync> actually holds Arc<Thread>
#   Thread.data: Box<dyn Any + Send + Sync> actually holds PosixThread


def _cast_boxed_dyn(box_val, concrete_type_name):
    concrete_type = lookup_type(concrete_type_name)
    if concrete_type is None:
        raise IntrospectionError(f"Cannot resolve type {concrete_type_name}")
    data_addr = int(box_val['pointer'])
    return gdb.Value(data_addr).cast(concrete_type.pointer()).dereference()


def get_thread_from_task(task_val):
    """Return the ``Thread`` held by ``Task.data``."""
    thread_arc = _cast_boxed_dyn(task_val['data'], TASK_DATA_CONCRETE)
    return unwrap_arc(thread_arc)


def get_posix_thread_from_thread(thread_val):
    """Return the ``PosixThread`` held by ``Thread.data``."""
    return _cast_boxed_dyn(thread_val['data'], THREAD_DATA_CONCRETE)


def get_posix_thread_from_task(task_val):
    """Return the ``PosixThread`` reached from a ``Task``."""
    return get_posix_thread_from_thread(get_thread_from_task(task_val))


# --- Process and thread row readers ---
#
# COUPLED: kernel/src/process/posix_thread/name.rs
#   ThreadName([u8; MAX_THREAD_NAME_LEN])


def _read_ppid(process_val):
    try:
        return read_scalar(process_val['parent']['pid'])
    except (gdb.error, IntrospectionError):
        return 0


def _read_status(process_val):
    try:
        status = process_val['status']
        if bool(read_scalar(status['is_zombie'])):
            return "Zombie"
        try:
            if bool(read_scalar(status['stop_status']['is_stopped'])):
                return "Stopped"
        except (gdb.error, IntrospectionError):
            pass
        return "Running"
    except (gdb.error, IntrospectionError):
        return "?"


def _read_thread_count(process_val):
    try:
        _, task_set = unwrap_mutex(process_val['tasks'])
        return int(task_set['tasks']['len'])
    except (gdb.error, IntrospectionError):
        return 0


def _read_thread_name(posix_thread):
    if posix_thread is None:
        return "<unknown>"
    try:
        _, name_val = unwrap_mutex(posix_thread['name'])
        try:
            buf = name_val['__0']
        except gdb.error:
            buf = name_val['0']

        bytes_out = bytearray()
        for idx in range(THREAD_NAME_MAX_LEN):
            byte = read_scalar(buf[idx])
            if byte == 0:
                break
            bytes_out.append(byte)
        return bytes_out.decode('utf-8', errors='replace') or "<unnamed>"
    except (gdb.error, IntrospectionError):
        return "<unknown>"


def _read_process_name(process_val):
    try:
        _, task_set = unwrap_mutex(process_val['tasks'])
        tasks = read_vec(task_set['tasks'])
        if tasks:
            first_task = unwrap_arc(tasks[0])
            return _read_thread_name(get_posix_thread_from_task(first_task))
    except (gdb.error, IntrospectionError):
        pass
    return "<unknown>"


def _read_tid(posix_thread):
    if posix_thread is None:
        return 0
    try:
        return read_scalar(posix_thread['tid'])
    except (gdb.error, IntrospectionError):
        return 0


def process_row(pid, process_val):
    """Return the display row for one process."""
    return ProcessRow(
        pid=pid,
        ppid=_read_ppid(process_val),
        state=_read_status(process_val),
        thread_count=_read_thread_count(process_val),
        name=_read_process_name(process_val),
    )


def iter_process_rows(filter_pid=None):
    """Yield process rows, optionally filtering by PID."""
    for pid, process in iter_processes():
        if filter_pid is not None and pid != filter_pid:
            continue
        yield process_row(pid, process)


def iter_thread_rows():
    """Yield thread rows for all live threads."""
    for tid_key, thread in iter_threads():
        posix_thread = get_posix_thread_from_thread(thread)
        tid = _read_tid(posix_thread) if posix_thread is not None else tid_key
        name = _read_thread_name(posix_thread)
        pid = 0
        if posix_thread is not None:
            process = unwrap_weak(posix_thread['process'])
            if process is not None:
                pid = read_scalar(process['pid'])

        yield ThreadRow(tid=tid, pid=pid, name=name)


def process_tree():
    """Return ``(roots, children, rows_by_pid)`` for process-tree output."""
    rows = list(iter_process_rows())
    rows_by_pid = {row.pid: row for row in rows}
    children = {}
    for row in rows:
        children.setdefault(row.ppid, []).append(row.pid)

    roots = [
        pid for pid, row in rows_by_pid.items()
        if row.ppid not in rows_by_pid
    ]
    if not roots and rows_by_pid:
        roots = sorted(rows_by_pid)[:1]
    return (sorted(roots), children, rows_by_pid)


# --- File table and file descriptor readers ---
#
# COUPLED: kernel/src/fs/file/file_table.rs, kernel/src/process/posix_thread/mod.rs
#   PosixThread.file_table: Mutex<Option<RoArc<FileTable>>>
#   RoArc<T> = Arc<Inner<T>>; Inner<T> { data: RwLock<T>, ... }


def _read_file_table_from_thread(posix_thread):
    _, ft_option = unwrap_mutex(posix_thread['file_table'])
    is_some, roarc = unwrap_option(ft_option)
    if not is_some:
        return None

    try:
        inner_arc = roarc['__0']
    except gdb.error:
        inner_arc = roarc['0']
    inner = unwrap_arc(inner_arc)
    try:
        return inner['data']['val']['value']
    except gdb.error:
        return inner['data']['value']


def get_file_table(process_val):
    """Return the ``FileTable`` for ``process_val`` or ``None``."""
    _, task_set = unwrap_mutex(process_val['tasks'])
    for task in read_vec(task_set['tasks']):
        posix_thread = get_posix_thread_from_task(unwrap_arc(task))
        if posix_thread is None:
            continue

        file_table = _read_file_table_from_thread(posix_thread)
        if file_table is not None:
            return file_table

    return None


def _read_fd_type(entry_val):
    try:
        vtable_ptr = entry_val['file']['vtable']['pointer']
        info = gdb.execute(f"info symbol {int(vtable_ptr)}", to_string=True)
        if "No symbol" in info:
            return "?"
        for marker in FILE_VTABLE_MARKERS:
            if marker in info:
                return marker
        if "vtable" in info:
            parts = info.split("::")
            for part in reversed(parts):
                cleaned = part.split("+")[0].strip().rstrip(">").rstrip(",")
                if cleaned and len(cleaned) > 2 and cleaned[0].isupper():
                    return cleaned
        return info.strip()[:40]
    except (gdb.error, IntrospectionError):
        return "?"


def fd_rows(pid):
    """Return file descriptor rows for a process PID."""
    process = find_process(pid)
    if process is None:
        raise LookupError(f"No process with PID {pid}")

    file_table = get_file_table(process)
    if file_table is None:
        raise IntrospectionError(f"Cannot read file table for PID {pid}")

    rows = []
    for fd, entry in read_slot_vec(file_table['table']):
        flags = read_scalar(entry['flags'])
        rows.append(
            FdRow(
                fd=fd,
                flags=flags,
                fd_type=_read_fd_type(entry),
                flag_text="O_CLOEXEC" if flags & 1 else "",
            )
        )
    return rows


# --- Time readers ---
#
# COUPLED: ostd/src/timer/mod.rs, ostd/src/timer/jiffies.rs


def uptime_snapshot():
    """Return the current kernel uptime snapshot."""
    elapsed = read_global(ELAPSED_JIFFIES_SYMBOL)
    if elapsed is None:
        raise IntrospectionError("cannot read ELAPSED jiffies")
    return UptimeSnapshot(
        jiffies=read_scalar(elapsed),
        frequency_hz=TIMER_FREQ_HZ,
    )


# --- GDB convenience functions ---


def _process_ptr(pid):
    try:
        process = find_process(int(pid))
    except (gdb.error, IntrospectionError) as error:
        raise gdb.GdbError(f"Cannot read PID table: {error}")
    if process is None:
        raise gdb.GdbError(f"No process with PID {int(pid)}")
    return gdb_bridge.to_pointer(process)


def _thread_ptr(tid):
    try:
        thread = find_thread(int(tid))
    except (gdb.error, IntrospectionError) as error:
        raise gdb.GdbError(f"Cannot read PID table: {error}")
    if thread is None:
        raise gdb.GdbError(f"No thread with TID {int(tid)}")
    return gdb_bridge.to_pointer(thread)


def _pid_table_ptr():
    try:
        return gdb_bridge.to_pointer(get_pid_table())
    except (gdb.error, IntrospectionError) as error:
        raise gdb.GdbError(f"Cannot read PID table: {error}")


def _file_table_ptr(pid):
    try:
        process = find_process(int(pid))
    except (gdb.error, IntrospectionError) as error:
        raise gdb.GdbError(f"Cannot read PID table: {error}")
    if process is None:
        raise gdb.GdbError(f"No process with PID {int(pid)}")

    try:
        table = get_file_table(process)
    except (gdb.error, IntrospectionError) as error:
        raise gdb.GdbError(f"Cannot navigate file table: {error}")
    if table is None:
        raise gdb.GdbError(f"No file table for PID {int(pid)}")
    return gdb_bridge.to_pointer(table)


def register():
    """Register kernel object convenience functions."""
    gdb_bridge.Function("ast_process", _process_ptr)
    gdb_bridge.Function("ast_thread", _thread_ptr)
    gdb_bridge.Function("ast_pid_table", _pid_table_ptr)
    gdb_bridge.Function("ast_file_table", _file_table_ptr)
