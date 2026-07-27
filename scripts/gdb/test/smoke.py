# SPDX-License-Identifier: MPL-2.0

"""
Smoke assertions for Asterinas GDB helpers.

The GDB command file handles connection and breakpoints.  This module
keeps the Python assertions readable and reusable.
"""

import gdb

from helper import kernel
from helper.constants import (
    ELAPSED_JIFFIES_SYMBOL,
    PID_TABLE_SYMBOL,
    TASK_DATA_CONCRETE,
    THREAD_DATA_CONCRETE,
)
from helper.layout import (
    IntrospectionError,
    lookup_type,
    read_global,
    read_scalar,
    read_vec,
    unwrap_atomic,
    unwrap_arc,
    unwrap_mutex,
    unwrap_rwmutex_value,
)


checks = []
HELPER_ERRORS = (gdb.error, IntrospectionError, LookupError)


def check(name, cond):
    checks.append((name, cond))


def has_field(value, field_name):
    try:
        value[field_name]
        return True
    except gdb.error:
        return False


def check_printer(name, value, expected_text, child_name=None,
                  expect_no_children=False):
    try:
        printer = gdb.default_visualizer(value)
        check(f"{name} has pretty-printer", printer is not None)
        if printer is None:
            return

        rendered = str(printer.to_string())
        check(f"{name} renders as {expected_text}", expected_text in rendered)

        children = list(printer.children())
        if child_name is not None:
            check(
                f"{name} exposes child {child_name}",
                any(child[0] == child_name for child in children),
            )
        if expect_no_children:
            check(f"{name} hides internal children", len(children) == 0)
    except Exception as error:
        gdb.write(f"Warning: {name} printer check failed: {error}\n")
        check(f"{name} pretty-printer", False)


def check_printer_registration():
    pp_out = gdb.execute("info pretty-printer", to_string=True)
    check("asterinas printer collection registered", "asterinas" in pp_out)
    check("AtomicScalar printer registered", "AtomicScalar" in pp_out)
    check("OstdMutex printer registered", "OstdMutex" in pp_out)
    check("OstdRwMutex printer registered", "OstdRwMutex" in pp_out)
    check("OstdSpinLock printer registered", "OstdSpinLock" in pp_out)


def check_kernel_symbols():
    try:
        pid_table_mutex = read_global(PID_TABLE_SYMBOL)
        check("PID_TABLE symbol resolves", pid_table_mutex is not None)
        if pid_table_mutex is not None:
            _locked, pid_table = unwrap_mutex(pid_table_mutex)
            check("PID_TABLE unwraps to entries", has_field(pid_table, "entries"))

        elapsed = read_global(ELAPSED_JIFFIES_SYMBOL)
        check("ELAPSED jiffies symbol resolves", elapsed is not None)
        if elapsed is not None:
            check("ELAPSED jiffies is readable", read_scalar(elapsed) >= 0)

        check("Task.data concrete type resolves",
              lookup_type(TASK_DATA_CONCRETE) is not None)
        check("Thread.data concrete type resolves",
              lookup_type(THREAD_DATA_CONCRETE) is not None)
    except HELPER_ERRORS as error:
        gdb.write(f"Warning: kernel symbol checks failed: {error}\n")
        check("kernel symbol checks", False)


def check_wrapper_printers():
    try:
        thread = kernel.find_thread(1)
        check("TID 1 is available for wrapper printer checks",
              thread is not None)
        posix_thread = kernel.get_posix_thread_from_thread(thread)

        tid = posix_thread['tid']
        tid_value = unwrap_atomic(tid)
        check("unwrap_atomic reads Atomic<u32> tid", tid_value == 1)
        check_printer("Atomic<u32>", tid, str(tid_value),
                      expect_no_children=True)

        is_exited = thread['is_exited']
        is_exited_value = bool(unwrap_atomic(is_exited))
        check("unwrap_atomic reads Atomic<bool> is_exited",
              is_exited_value is False)
        check_printer(
            "Atomic<bool>",
            is_exited,
            "true" if is_exited_value else "false",
            expect_no_children=True,
        )

        timer_slack = posix_thread['timer_slack_ns']
        timer_slack_value = unwrap_atomic(timer_slack)
        check("unwrap_atomic reads Atomic<u64> timer_slack_ns",
              timer_slack_value >= 0)
        check_printer("Atomic<u64>", timer_slack, str(timer_slack_value),
                      expect_no_children=True)

        thread_name = posix_thread['name']
        locked, _name_value = unwrap_mutex(thread_name)
        check("unwrap_mutex reads PosixThread.name", isinstance(locked, bool))
        check_printer(
            "Mutex<ThreadName>",
            thread_name,
            f"Mutex(locked={'true' if locked else 'false'})",
            child_name="value",
        )

        thread_fs = posix_thread['fs']
        _thread_fs_value = unwrap_rwmutex_value(thread_fs)
        check_printer("RwMutex<ThreadFsInfo>", thread_fs, "RwMutex",
                      child_name="value")

        signalled_waker = posix_thread['signalled_waker']
        check_printer("SpinLock<Option<Waker>>", signalled_waker,
                      "SpinLock(locked=", child_name="value")

        atomic_out = gdb.execute("p (*$ast_thread(1)).is_exited",
                                 to_string=True)
        check(
            "GDB print uses Atomic<bool> pretty-printer",
            ("false" in atomic_out or "true" in atomic_out)
            and "UnsafeCell" not in atomic_out,
        )
    except Exception as error:
        gdb.write(f"Warning: wrapper printer checks failed: {error}\n")
        check("wrapper printer checks", False)


def check_direct_prints():
    try:
        pid_table = gdb.execute(
            "p aster_kernel::process::pid_table::PID_TABLE",
            to_string=True,
        )
        check("PID_TABLE renders with Mutex prefix",
              "Mutex(locked=" in pid_table)
    except gdb.error:
        check("PID_TABLE accessible", False)

    try:
        bootstrap = gdb.execute("p ostd::IN_BOOTSTRAP_CONTEXT",
                                to_string=True)
        check("IN_BOOTSTRAP_CONTEXT renders as bool",
              "false" in bootstrap or "true" in bootstrap)
        check("IN_BOOTSTRAP_CONTEXT no UnsafeCell",
              "UnsafeCell" not in bootstrap)
    except gdb.error:
        check("IN_BOOTSTRAP_CONTEXT accessible", False)

    try:
        hwmap = gdb.execute("p ostd::boot::smp::HW_CPU_ID_MAP",
                            to_string=True)
        check("HW_CPU_ID_MAP renders with SpinLock prefix",
              "SpinLock(locked=" in hwmap)
    except gdb.error:
        check("HW_CPU_ID_MAP accessible (optional)", True)


def check_convenience_functions():
    try:
        pid_table = gdb.execute("p *$ast_pid_table()", to_string=True)
        check("$ast_pid_table() returns PidTable",
              "entries" in pid_table or "process_count" in pid_table)
    except gdb.error:
        check("$ast_pid_table() invocation", False)

    try:
        proc1 = gdb.execute("p *$ast_process(1)", to_string=True)
        check("$ast_process(1) returns Process", "pid" in proc1.lower())
    except gdb.error:
        check("$ast_process(1) invocation", False)

    try:
        thread1 = gdb.execute("p *$ast_thread(1)", to_string=True)
        check(
            "$ast_thread(1) returns Thread",
            "data" in thread1
            or "status" in thread1
            or "tid" in thread1.lower(),
        )
    except gdb.error:
        check("$ast_thread(1) invocation", False)

    try:
        ft1 = gdb.execute("p *$ast_file_table(1)", to_string=True)
        check(
            "$ast_file_table(1) returns FileTable",
            "table" in ft1 or "slots" in ft1 or "fds" in ft1.lower(),
        )
    except gdb.error:
        check("$ast_file_table(1) invocation", False)


def check_kernel_navigation():
    try:
        pid_table = kernel.get_pid_table()
        check("kernel.get_pid_table reads entries",
              has_field(pid_table, "entries"))

        process = kernel.find_process(1)
        check("kernel.find_process(1) returns Process", process is not None)
        check("kernel.find_process missing PID returns None",
              kernel.find_process(999999) is None)

        thread = kernel.find_thread(1)
        check("kernel.find_thread(1) returns Thread", thread is not None)
        check("kernel.find_thread missing TID returns None",
              kernel.find_thread(999999) is None)

        row = kernel.process_row(1, process)
        check("kernel.process_row reads PID", row.pid == 1)
        check("kernel.process_row reads thread count", row.thread_count >= 1)
        check("kernel.process_row reads name", bool(row.name))

        _locked, task_set = unwrap_mutex(process['tasks'])
        tasks = read_vec(task_set['tasks'])
        check("Process.tasks contains a task", bool(tasks))
        if tasks:
            task = unwrap_arc(tasks[0])
            task_thread = kernel.get_thread_from_task(task)
            posix_thread = kernel.get_posix_thread_from_task(task)
            check("Task.data downcasts to Thread",
                  int(task_thread.address) == int(thread.address))
            check("Task.data reaches PosixThread", posix_thread is not None)

        file_table = kernel.get_file_table(process)
        check("kernel.get_file_table(1) returns FileTable",
              file_table is not None)
    except HELPER_ERRORS as error:
        gdb.write(f"Warning: kernel navigation checks failed: {error}\n")
        check("kernel navigation checks", False)


def check_commands():
    try:
        rows = list(kernel.iter_process_rows())
        check("kernel.iter_process_rows sees PID 1",
              any(row.pid == 1 for row in rows))

        ps = gdb.execute("ast-ps", to_string=True)
        check("ast-ps has header", "PID" in ps and "PPID" in ps)
        check("ast-ps lists PID 1",
              any(line.split() and line.split()[0] == "1"
                  for line in ps.splitlines()))

        ps_one = gdb.execute("ast-ps 1", to_string=True)
        check("ast-ps 1 lists PID 1",
              any(line.split() and line.split()[0] == "1"
                  for line in ps_one.splitlines()))
    except HELPER_ERRORS:
        check("ast-ps runs", False)

    try:
        thread_rows = list(kernel.iter_thread_rows())
        check("kernel.iter_thread_rows returns rows", bool(thread_rows))

        threads = gdb.execute("ast-threads", to_string=True)
        check("ast-threads has header", "TID" in threads)
    except HELPER_ERRORS:
        check("ast-threads runs", False)

    try:
        roots, _children, rows_by_pid = kernel.process_tree()
        check("kernel.process_tree returns PID 1", 1 in rows_by_pid)
        check("kernel.process_tree has roots", bool(roots))

        pstree = gdb.execute("ast-pstree", to_string=True)
        check("ast-pstree lists PID 1", "(1)" in pstree)
    except HELPER_ERRORS:
        check("ast-pstree runs", False)

    try:
        uptime = kernel.uptime_snapshot()
        check("kernel.uptime_snapshot reads jiffies", uptime.jiffies >= 0)

        uptime_out = gdb.execute("ast-uptime", to_string=True)
        check("ast-uptime reports jiffies",
              "jiffies" in uptime_out and "Hz" in uptime_out)
    except HELPER_ERRORS:
        check("ast-uptime runs", False)

    try:
        fd_rows = kernel.fd_rows(1)
        check("kernel.fd_rows returns a list", isinstance(fd_rows, list))

        fds = gdb.execute("ast-fds 1", to_string=True)
        check("ast-fds for PID 1 has header", "FD" in fds and "TYPE" in fds)
        fds_usage = gdb.execute("ast-fds", to_string=True)
        check("ast-fds without PID reports usage", "Usage:" in fds_usage)
    except HELPER_ERRORS:
        check("ast-fds runs", False)

    try:
        version = gdb.execute("ast-version", to_string=True)
        check("ast-version reports Asterinas", "Asterinas" in version)
    except gdb.error:
        check("ast-version runs", False)


def run():
    check_printer_registration()
    check_kernel_symbols()
    check_wrapper_printers()
    check_direct_prints()
    check_convenience_functions()
    check_kernel_navigation()
    check_commands()

    failures = [name for name, ok in checks if not ok]
    if failures:
        gdb.write(f"SMOKE: fail: {', '.join(failures)}\n")
    else:
        gdb.write("SMOKE: all ok\n")
