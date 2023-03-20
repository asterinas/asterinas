#!/bin/bash

# This script is partly from occlum

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
TAG=20230127
LTP_SRC_DIR=${SCRIPT_DIR}/ltp
JINUX_SYSCALL_LIST=${SCRIPT_DIR}/syscall-jinux
SKIP_LIST=${SCRIPT_DIR}/skip-list
STATIC_CFLAGS="CFLAGS += -static -pthread" 

# download and config ltp
if [ ! -d "ltp" ]; then
    git clone -b ${TAG} https://github.com/linux-test-project/ltp.git
    pushd ${LTP_SRC_DIR}
    make autotools
    ./configure
    popd
fi

# build testcases for syscalls
pushd ${LTP_SRC_DIR}/testcases/kernel/syscalls
for SYSCALL in $( ls );
do
    if [ -d ${SYSCALL} ] && grep -q "${SYSCALL}" ${JINUX_SYSCALL_LIST} && ! grep -q "${SYSCALL}" ${SKIP_LIST}; then
        echo ${SYSCALL}
        pushd ${SYSCALL} > /dev/null
        TESTS=
        for SRC in $(ls *.c);
        do
            TESTS="${TESTS} ${SRC/.c/}" 
        done
        if [ -n "${TESTS}" ]; then
            MAKE_FLAGS=$(echo "${TESTS}: ${STATIC_CFLAGS}" | awk '$1=$1')
            if ! grep -q "${MAKE_FLAGS}" Makefile; then 
                echo "${MAKE_FLAGS}" >> Makefile
            fi
        fi
        make clean > /dev/null
        make >/dev/null 2>&1
        for TEST in ${TESTS}; 
        do
            FILE_INFO=$(file ${TEST})
            # echo echo ${FILE_INFO}
            if echo ${FILE_INFO} | grep -q -v "statically linked"; then
                echo "compile ${TEST} failed".
                exit 1
            fi
        done
        popd > /dev/null
    fi
done
popd