#!/usr/bin/env sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Match `ttyS0`'s baud rate and emit LF-only lines for `grep -Fxq` in `run.sh`.
stty 115200 -onlcr

report_status() {
    echo "Linux regression exit status: $1"
    # Re-apply `-onlcr` (`TCSADRAIN`) to flush pending UART output before hangup.
    stty -onlcr || true
}
trap 'exit 1' HUP INT TERM
trap 'report_status "$?"' EXIT

# Installed from `test/initramfs/src/regression/scripts/run_regression_test.sh`.
/test/run_regression_test.sh
