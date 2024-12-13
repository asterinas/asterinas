#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

RUNTIME=90
REPORT_INTERVAL=$((RUNTIME+10))

/benchmark/bin/schbench -F 256 -n 5 -r $RUNTIME -i $REPORT_INTERVAL
