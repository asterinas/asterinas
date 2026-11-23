# SPDX-License-Identifier: MPL-2.0

"""
Small bridge helpers around the GDB Python API.

The rest of the helpers should speak in kernel objects and rows.  This
module keeps user-interface details such as pointer conversion, argument
parsing, table formatting, and diagnostic printing in one place.
"""

import gdb


class Command(gdb.Command):
    """Thin adapter from a Python handler to a GDB command."""

    def __init__(self, name, handler):
        super().__init__(name, gdb.COMMAND_USER)
        self._handler = handler

    def invoke(self, arg, from_tty):
        self._handler(arg)


class Function(gdb.Function):
    """Thin adapter from a Python handler to a GDB convenience function."""

    def __init__(self, name, handler):
        super().__init__(name)
        self._handler = handler

    def invoke(self, *args):
        return self._handler(*args)


def to_pointer(value):
    """Return a pointer-like ``gdb.Value`` for a memory-backed value."""
    addr = int(value.address) if value.address is not None else None
    if addr is None:
        return value
    return gdb.Value(addr).cast(value.type.pointer())


def parse_int_arg(arg, name):
    """Parse a required integer command argument."""
    text = arg.strip()
    if not text:
        raise ValueError(f"missing {name}")
    try:
        return int(text)
    except ValueError as error:
        raise ValueError(f"invalid {name} '{text}'") from error


def write_error(message):
    """Print a command error in the normal helper style."""
    gdb.write(f"Error: {message}\n")


def _format_cell(value, width, align):
    text = str(value)
    if width is None:
        return text
    if align == "<":
        return text.ljust(width)
    return text.rjust(width)


def write_table(columns, rows):
    """Write aligned table output.

    ``columns`` is a sequence of ``(header, width, align)`` tuples.
    ``align`` is ``">"`` or ``"<"``.
    """
    gdb.write(
        "  ".join(
            _format_cell(header, width, align)
            for header, width, align in columns
        )
        + "\n"
    )
    for row in rows:
        gdb.write(
            "  ".join(
                _format_cell(value, width, align)
                for value, (_header, width, align) in zip(row, columns)
            )
            + "\n"
        )
