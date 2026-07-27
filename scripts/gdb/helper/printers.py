# SPDX-License-Identifier: MPL-2.0

"""
GDB pretty-printers for Asterinas wrapper types.

Registers printers for:
- core::sync::atomic::Atomic<T>
- ostd::sync::Mutex<T>
- ostd::sync::RwMutex<T>
- ostd::sync::SpinLock<T, G>

These printers collapse the noisy internal layout (UnsafeCell, etc.)
into readable one-line representations, making every GDB display
command (``p``, ``bt full``, ``info locals``, watchpoints, IDE panels)
useful out of the box.
"""

import gdb.printing
import gdb

from helper.layout import IntrospectionError, unwrap_atomic


class AtomicPrinter:
    """Collapse Atomic<T> storage wrappers to just N."""

    def __init__(self, val):
        self.val = val

    def to_string(self):
        try:
            raw = unwrap_atomic(self.val)
            type_name = str(self.val.type)
            if "Atomic<bool>" in type_name:
                return "true" if raw else "false"
            return str(raw)
        except (IntrospectionError, gdb.error):
            return None

    def children(self):
        return []


class MutexPrinter:
    """Show Mutex(locked=<bool>) = <inner T>."""

    def __init__(self, val):
        self.val = val

    def to_string(self):
        try:
            locked = bool(unwrap_atomic(self.val['lock']))
            return f"Mutex(locked={'true' if locked else 'false'})"
        except (IntrospectionError, gdb.error):
            return None

    def children(self):
        try:
            inner = self.val['val']['value']
            return [("value", inner)]
        except gdb.error:
            return []


class RwMutexPrinter:
    """Show RwMutex = <inner T>.

    The lock field is an AtomicUsize with a complex bitfield encoding
    (reader count, writer flag, upgrade state).  We intentionally do
    not parse it; just show the inner value.
    """

    def __init__(self, val):
        self.val = val

    def to_string(self):
        return "RwMutex"

    def children(self):
        try:
            inner = self.val['val']['value']
            return [("value", inner)]
        except gdb.error:
            return []


class SpinLockPrinter:
    """Show SpinLock(locked=<bool>) = <inner T>.

    SpinLock<T, G> is #[repr(transparent)] over SpinLockInner<T>,
    so we traverse val['inner']['val']['value'] for the inner T
    and val['inner']['lock'] for the lock state.
    """

    def __init__(self, val):
        self.val = val

    def to_string(self):
        try:
            locked = bool(unwrap_atomic(self.val['inner']['lock']))
            return f"SpinLock(locked={'true' if locked else 'false'})"
        except (IntrospectionError, gdb.error):
            return None

    def children(self):
        try:
            inner = self.val['inner']['val']['value']
            return [("value", inner)]
        except gdb.error:
            return []


def build_printer():
    """Build and return the Asterinas pretty-printer collection."""
    pp = gdb.printing.RegexpCollectionPrettyPrinter("asterinas")
    pp.add_printer(
        "AtomicScalar",
        r"^core::sync::atomic::Atomic<.*>$",
        AtomicPrinter,
    )
    pp.add_printer(
        "OstdMutex",
        r"^ostd::sync::(.*::)?Mutex<.*>$",
        MutexPrinter,
    )
    pp.add_printer(
        "OstdRwMutex",
        r"^ostd::sync::(.*::)?RwMutex<.*>$",
        RwMutexPrinter,
    )
    pp.add_printer(
        "OstdSpinLock",
        r"^ostd::sync::(.*::)?SpinLock<.*>$",
        SpinLockPrinter,
    )
    return pp


def register():
    """Register the Asterinas printers with GDB."""
    objfile = gdb.current_objfile()
    scope = objfile if objfile is not None else gdb.current_progspace()
    gdb.printing.register_pretty_printer(
        scope, build_printer(), replace=True
    )
