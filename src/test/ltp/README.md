## Intro
This directory contains several scripts and configuration files to enable running LTP(https://github.com/linux-test-project/ltp) on jinux. 

## About scripts and configuration files
- syscall-jinux: This file defines the syscalls that we want to build and run ltp tests on
- skip-list: This file defines the syscalls that will not be tested. 
- download_and_build_ltp.sh: download ltp from github and compile tests on syscalls specified in syscall-jinux.
- copy_tests.sh: copy compiled tests from ltp source code to a ltp_test dir, and generate a runltp script that can run in jinux
- update_syscall_list.sh: update syscall-jinux to include all syscalls jinux implements(by scanning syscall/mod.rs).

## Limitations:
1. This is only a temporary solution to running ltp now. We cannot use the scripts that ltp provides. 
2. Since all testcases are statically linked, the testcases are rather large. We cannot copy two many testcases to initramfs since jinux only have very limited memory now.