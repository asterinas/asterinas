#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

sh /benchmark/fio/common/run_fio.sh randread 10485760
