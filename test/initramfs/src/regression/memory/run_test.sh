#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

case "$0" in
*/*) cd "${0%/*}" ;;
esac

./mmap/mmap_and_fork
./mmap/mmap_and_mprotect
./mmap/mmap_and_mremap
./mmap/mmap_beyond_the_file
./mmap/mmap_err
./mmap/mmap_holes
./mmap/mmap_populate
./mmap/mmap_readahead
./mmap/mmap_shared_filebacked
./mmap/mmap_vmrss
./mmap/rev_map_ext2
./mmap/rev_map_tmpfs
