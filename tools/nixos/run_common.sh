#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# TODO: When fully supporting ACPI shutdown, remove this script, 
# execute the QEMU command and return the exit code directly.

# The kernel uses a specific value to signal a successful shutdown via the
# isa-debug-exit device.
KERNEL_SUCCESS_EXIT_CODE=16 # 0x10 in hexadecimal
# QEMU translates the value written to the isa-debug-exit device into a final
# process exit code using following formula.
QEMU_SUCCESS_EXIT_CODE=$(((KERNEL_SUCCESS_EXIT_CODE << 1) | 1))

"$@" || exit_code=$?
exit_code=${exit_code:-0}

if [ ${exit_code} -eq 0 ] || [ ${exit_code} -eq ${QEMU_SUCCESS_EXIT_CODE} ]; then
    exit 0
fi

exit ${exit_code}