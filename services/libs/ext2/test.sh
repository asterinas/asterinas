#!/bin/bash

set -e

rm -f /root/ext2.image
dd if=/dev/zero of=/root/ext2.image bs=1G count=1
mke2fs /root/ext2.image

RUST_BACKTRACE=1 cargo test -- --nocapture
