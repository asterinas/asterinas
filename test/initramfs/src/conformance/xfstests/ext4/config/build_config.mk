# SPDX-License-Identifier: MPL-2.0

XFSTESTS_NEEDS_BLOCK_DEVICES := true
XFSTESTS_MKFS := mkfs.ext4 -F -b 4096 -I 256 -O extent,filetype,^has_journal,^metadata_csum,^64bit,^flex_bg,^inline_data,^resize_inode,^dir_index,^huge_file,^dir_nlink,^extra_isize
