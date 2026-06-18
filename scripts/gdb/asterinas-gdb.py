# SPDX-License-Identifier: MPL-2.0

"""
Asterinas GDB Debug Helper Scripts -- Entry Point

This file is sourced by GDB (via .gdbinit or manually) to register
Asterinas-specific pretty-printers, convenience functions, and commands.

Usage:
    (gdb) source scripts/gdb/asterinas-gdb.py
"""

# Add scripts/gdb/ to sys.path first so the `helper` package is
# importable.  This must happen before `import gdb` because later
# code does `from helper.xxx import ...` which requires the path
# to be set up.
import sys
import os

_script_dir = os.path.dirname(os.path.abspath(__file__))
if _script_dir not in sys.path:
    sys.path.insert(0, _script_dir)

import gdb  # noqa: E402  (must come after sys.path setup)


def _has_pretty_printer(scope, name):
    """Return whether ``scope`` already has a named pretty-printer."""
    return any(
        getattr(printer, "name", None) == name
        for printer in getattr(scope, "pretty_printers", [])
    )


def _check_rust_pretty_printers():
    """Register the rust-gdb stdlib pretty-printers if available.

    The Asterinas kernel ELF is built for ``x86_64-unknown-none`` and
    therefore lacks the ``.debug_gdb_scripts`` section that normally
    triggers automatic loading of the Rust standard-library printers.
    Even when launched via ``rust-gdb`` (which sets ``PYTHONPATH`` to
    include ``$rustlib/etc``), the printers stay dormant unless we
    explicitly call ``register_printers``.
    """
    try:
        import gdb_lookup
    except ImportError:
        return False
    try:
        objfile = gdb.current_objfile()
        progspace = gdb.current_progspace()
        if (
            (objfile is not None and _has_pretty_printer(objfile, "rust"))
            or _has_pretty_printer(progspace, "rust")
        ):
            return True

        gdb_lookup.register_printers(
            objfile if objfile is not None else progspace
        )
    except Exception:
        try:
            progspace = gdb.selected_inferior().progspace
            if _has_pretty_printer(progspace, "rust"):
                return True
            gdb_lookup.register_printers(progspace)
        except Exception:
            return False
    return True


class AstVersion(gdb.Command):
    """Print Asterinas kernel version."""

    def __init__(self):
        super().__init__("ast-version", gdb.COMMAND_USER)

    def invoke(self, arg, from_tty):
        version_file = os.path.join(_script_dir, "..", "..", "VERSION")
        try:
            with open(version_file) as f:
                gdb.write(f"Asterinas {f.read().strip()}\n")
        except OSError:
            gdb.write("Asterinas (version unknown)\n")


def _register():
    """Register all helpers."""
    AstVersion()

    loaded = []
    warnings = []
    for name in ("printers", "kernel", "commands"):
        try:
            mod = __import__(f"helper.{name}", fromlist=[name])
            if hasattr(mod, "register"):
                mod.register()
            loaded.append(name)
        except Exception as e:
            warnings.append(f"{name}: {e}")
    return loaded, warnings


if getattr(gdb, "_asterinas_helpers_loaded", False):
    gdb.write("Asterinas GDB helpers already loaded.\n")
else:
    _rust_pp = _check_rust_pretty_printers()
    _loaded, _warnings = _register()
    gdb._asterinas_helpers_loaded = True

    if _warnings:
        for w in _warnings:
            gdb.write(f"Warning: failed to load helper module: {w}\n")

    if not _rust_pp:
        gdb.write(
            "Warning: rust-gdb pretty-printers not detected.\n"
            "  The Asterinas helpers work best with rust-gdb.\n"
            "  Use `cargo osdk debug` or launch `rust-gdb` directly.\n"
        )

    _tag = "rust-pp: active" if _rust_pp else "rust-pp: off"
    gdb.write(
        f"Asterinas GDB helpers loaded "
        f"({', '.join(_loaded) if _loaded else 'core only'}). "
        f"[{_tag}]. "
        "Type 'ast-version' to check kernel version.\n"
    )
