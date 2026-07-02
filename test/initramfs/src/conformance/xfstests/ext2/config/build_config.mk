# SPDX-License-Identifier: MPL-2.0

# Whether block device images need to be created for test and scratch partitions.
XFSTESTS_NEEDS_BLOCK_DEVICES := true
# The mkfs command used to format block device images (empty if not applicable).
XFSTESTS_MKFS := mkfs.ext2
