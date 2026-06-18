# SPDX-License-Identifier: MPL-2.0

"""
GDB user commands for Asterinas inspection.

Commands only parse arguments and format rows.  The Rust layout knowledge
is kept in ``helper.kernel``.
"""

import gdb

from helper import gdb_bridge, kernel
from helper.layout import IntrospectionError


def ast_ps(arg):
    try:
        filter_pid = (
            gdb_bridge.parse_int_arg(arg, "PID") if arg.strip() else None
        )
        rows = list(kernel.iter_process_rows(filter_pid))
        gdb_bridge.write_table(
            (
                ("PID", 6, ">"),
                ("PPID", 6, ">"),
                ("STATE", 10, "<"),
                ("THREADS", 7, ">"),
                ("NAME", None, "<"),
            ),
            ((row.pid, row.ppid, row.state, row.thread_count, row.name)
             for row in rows),
        )
        if filter_pid is not None and not rows:
            gdb.write(f"No process with PID {filter_pid}\n")
    except ValueError as error:
        gdb_bridge.write_error(error)
    except (gdb.error, IntrospectionError) as error:
        gdb_bridge.write_error(f"cannot read PID table: {error}")


def ast_threads(arg):
    try:
        gdb_bridge.write_table(
            (("TID", 6, ">"), ("PID", 6, ">"), ("NAME", None, "<")),
            ((row.tid, row.pid, row.name) for row in kernel.iter_thread_rows()),
        )
    except (gdb.error, IntrospectionError) as error:
        gdb_bridge.write_error(f"cannot read PID table: {error}")


def ast_pstree(arg):
    try:
        roots, children, rows_by_pid = kernel.process_tree()
        if not rows_by_pid:
            gdb.write("No processes found\n")
            return

        def walk(pid, prefix, is_last):
            row = rows_by_pid[pid]
            connector = "`-- " if is_last else "|-- "
            gdb.write(f"{prefix}{connector}{row.name}({pid}) [{row.state}]\n")
            child_prefix = prefix + ("    " if is_last else "|   ")
            kids = sorted(children.get(pid, []))
            for idx, child in enumerate(kids):
                walk(child, child_prefix, idx == len(kids) - 1)

        for root in roots:
            row = rows_by_pid[root]
            gdb.write(f"{row.name}({root}) [{row.state}]\n")
            kids = sorted(children.get(root, []))
            for idx, child in enumerate(kids):
                walk(child, "", idx == len(kids) - 1)
    except (gdb.error, IntrospectionError) as error:
        gdb_bridge.write_error(f"cannot read PID table: {error}")


def ast_fds(arg):
    try:
        target_pid = gdb_bridge.parse_int_arg(arg, "PID")
        rows = kernel.fd_rows(target_pid)
        gdb_bridge.write_table(
            (
                ("FD", 4, ">"),
                ("FLAGS", 5, ">"),
                ("TYPE", None, "<"),
                ("", None, "<"),
            ),
            ((row.fd, row.flags, row.fd_type, row.flag_text) for row in rows),
        )
    except ValueError:
        gdb.write("Usage: ast-fds <PID>\n")
    except LookupError as error:
        gdb.write(f"{error}\n")
    except (gdb.error, IntrospectionError) as error:
        gdb_bridge.write_error(error)


def ast_uptime(arg):
    try:
        uptime = kernel.uptime_snapshot()
        total_secs = uptime.jiffies // uptime.frequency_hz
        millis = uptime.jiffies % uptime.frequency_hz
        hours = total_secs // 3600
        mins = (total_secs % 3600) // 60
        secs = total_secs % 60
        gdb.write(
            f"Uptime: {hours:02d}:{mins:02d}:{secs:02d}.{millis:03d}  "
            f"({uptime.jiffies} jiffies, {uptime.frequency_hz} Hz)\n"
        )
    except (gdb.error, IntrospectionError) as error:
        gdb_bridge.write_error(error)


def register():
    """Register all user commands."""
    gdb_bridge.Command("ast-ps", ast_ps)
    gdb_bridge.Command("ast-threads", ast_threads)
    gdb_bridge.Command("ast-pstree", ast_pstree)
    gdb_bridge.Command("ast-fds", ast_fds)
    gdb_bridge.Command("ast-uptime", ast_uptime)
