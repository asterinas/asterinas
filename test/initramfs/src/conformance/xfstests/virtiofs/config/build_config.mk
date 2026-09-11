# SPDX-License-Identifier: MPL-2.0

# Virtio-fs uses host directories rather than block device images.
XFSTESTS_NEEDS_BLOCK_DEVICES := false
XFSTESTS_MKFS :=
