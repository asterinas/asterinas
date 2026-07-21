#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

VIRTIOFS_TAG=${VIRTIOFS_TAG:-aster-virtiofs}
FIO_MOUNT_POINT=${FIO_MOUNT_POINT:-/virtiofs}
FIO_TEST_FILE=${FIO_TEST_FILE:-${FIO_MOUNT_POINT}/fio-test}
