#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

FIO_FS_TYPE=ext2 FIO_WORKLOAD=read /benchmark/fio/common/run.sh
