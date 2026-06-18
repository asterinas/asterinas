# Asterinas GDB pretty-printer smoke test
#
# Run via: rust-gdb --batch --command=scripts/gdb/test/smoke.gdb <kernel-elf>
# The wrapper starts QEMU and waits for the OSDK GDB socket.

# Connect and source helpers
target remote .osdk-gdb-socket
source scripts/gdb/asterinas-gdb.py

# The QEMU GDB server starts at the x86 reset vector (BIOS running,
# paging not yet enabled, kernel ELF not loaded into guest memory).
# First, break at the kernel's ostd entry point so paging is set up.
# Then advance to the first syscall handler so the init process has
# been spawned and PID 1 exists in the PID table.
hbreak __ostd_main
continue
delete
hbreak aster_kernel::syscall::handle_syscall
continue
delete

# Run smoke assertions
python
import sys
sys.path.insert(0, "scripts/gdb/test")
import smoke
smoke.run()
end

quit
