# SPDX-License-Identifier: MPL-2.0

# xfstests needs both the test and scratch virtio-fs mounts.
VIRTIOFS := on
VIRTIOFS_SCRATCH := on
XFSTESTS_TEST_DEV ?= aster-virtiofs
XFSTESTS_SCRATCH_DEV ?= aster-virtiofs-scratch

# Virtio-fs uses host directories rather than block device images.
XFSTESTS_NEEDS_BLOCK_DEVICES := false
XFSTESTS_MKFS :=
