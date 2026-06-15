#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

VIRTIOFS_TAG=aster-virtiofs FIO_FS_TYPE=virtiofs FIO_WORKLOAD=write /benchmark/fio/common/run.sh
