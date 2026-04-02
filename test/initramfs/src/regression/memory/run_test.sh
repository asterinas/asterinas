#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./mmap/mmap_and_fork
./mmap/mmap_and_mprotect
./mmap/mmap_and_mremap
./mmap/mmap_beyond_the_file
./mmap/mmap_err
./mmap/mmap_holes
./mmap/mmap_readahead
./mmap/mmap_shared_filebacked
./mmap/mmap_vmrss
