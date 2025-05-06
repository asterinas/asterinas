#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

CREATE_ENTRIES=1 /opt/ltp/runltp -f syscalls -W /tmp/zoo -p -l /tmp/result
cat /tmp/result
if grep -q "Total Failures: 0" /tmp/result; then
    exit 0
else
    exit 1
fi
