#!/bin/bash

# This file is used to update syscall-jinux to contain all syscalls that jinux already implements.

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
SYSCALL_MOD_DIR=$( cd ${SCRIPT_DIR}/../../services/libs/jinux-std/src/syscall && pwd)
SYSCALL_MOD_FILE=${SYSCALL_MOD_DIR}/mod.rs
SYSCALL_JINUX=${SCRIPT_DIR}/syscall-jinux

if [ ! -f ${SYSCALL_MOD_FILE} ]; then
    echo "cannot find syscall definition file"
    exit 1
fi

if [ -f ${SYSCALL_JINUX} ]; then
    rm ${SYSCALL_JINUX}
fi

for SYSCALL_DEF in $(grep "SYS_" ${SYSCALL_MOD_FILE} | grep -v "syscall_handler")
do
    if [[ ${SYSCALL_DEF} == SYS_* ]]; then
        SYSCALL_NAME=$(echo ${SYSCALL_DEF/SYS_/} | tr '[:upper:]' '[:lower:]')
        echo ${SYSCALL_NAME} >> ${SYSCALL_JINUX}
    fi
done
