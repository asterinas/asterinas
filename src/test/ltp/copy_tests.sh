#!/bin/bash

# This file will clone testfiles to jinux and generate a runltp script to run all tests.

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
SYSCALL_JINUX=${SCRIPT_DIR}/syscall-jinux
LTP_SYSCALL_DIR=${SCRIPT_DIR}/ltp/testcases/kernel/syscalls
SKIP_LIST=${SCRIPT_DIR}/skip-list
TARGET_DIR=${SCRIPT_DIR}/ltp_test
TEST_SCRIPT=${TARGET_DIR}/runltp

SYSCALL_LIST=
if [ $# -eq 1 ]; then
    SYSCALL_LIST=$1
else
    if [ ! -f ${SYSCALL_JINUX} ]; then
        echo "${SYSCALL_JINUX} does not exist".
        exit 1
    fi
    SYSCALL_LIST=$(cat ${SYSCALL_JINUX})
fi

rm -rf ${TARGET_DIR}
mkdir ${TARGET_DIR}

rm -f ${TEST_SCRIPT}
echo "#!/bin/sh" > ${TEST_SCRIPT}
echo "set -e" >> ${TEST_SCRIPT}
# FIXME: hardcode path here since we donot have pipe support.
echo "cd /test/ltp" >> ${TEST_SCRIPT}

for SYS_CALL in ${SYSCALL_LIST};
do 
    if ! grep -q ${SYS_CALL} ${SKIP_LIST}; then 
        echo ${SYS_CALL}
        for TEST_CASE in $( ls ${LTP_SYSCALL_DIR}/${SYS_CALL}/${SYS_CALL}[0-9]* 2>/dev/null);
        do 
            if [[ ${TEST_CASE} != *.c && ${TEST_CASE} != *.h ]]; then
                cp ${TEST_CASE} ${TARGET_DIR}
                TEST_NAME=$( basename ${TEST_CASE} )
                echo "./${TEST_NAME}" >> ${TEST_SCRIPT}
            fi
        done
    fi
done

echo "cd -" >> ${TEST_SCRIPT}

chmod +x ${TEST_SCRIPT}
