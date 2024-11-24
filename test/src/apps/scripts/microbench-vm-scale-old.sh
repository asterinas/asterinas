#!/bin/sh

set -ex

/test/scale/mmap
/test/scale/mmap_pf
/test/scale/pf_dist
/test/scale/pf_rand
/test/scale/munmap_virt
/test/scale/munmap_dist

poweroff -f
