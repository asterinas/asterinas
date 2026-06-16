#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./clock_nanosleep/nanosleep_err

./gettimeofday/gettimeofday

./itimer/setitimer
./itimer/timer_create
