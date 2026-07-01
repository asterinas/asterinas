#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

CONFORMANCE_TEST_SUITE=${CONFORMANCE_TEST_SUITE:-ltp}
CONFORMANCE_BLOCKLIST_MODE=${CONFORMANCE_BLOCKLIST_MODE:-auto}
CONFORMANCE_TESTS=${CONFORMANCE_TESTS:-}
LTP_DIR=/opt/ltp
GVISOR_DIR=/opt/gvisor
KSELFTEST_DIR=/opt/kselftest
XFSTESTS_DIR=/opt/xfstests

should_apply_blocklists() {
    case "$CONFORMANCE_BLOCKLIST_MODE" in
        apply)  return 0 ;;
        ignore) return 1 ;;
        auto|*) [ -z "$CONFORMANCE_TESTS" ] ;;
    esac
}

if should_apply_blocklists; then
    export CONFORMANCE_APPLY_BLOCKLISTS=1
else
    export CONFORMANCE_APPLY_BLOCKLISTS=0
fi

if [ "${CONFORMANCE_TEST_SUITE}" = "ltp" ]; then
    echo "Running LTP syscall tests..."
    if ! "${LTP_DIR}/run_ltp_test.sh"; then
        echo "Error: LTP syscall tests failed." >&2
        exit 1
    fi
elif [ "${CONFORMANCE_TEST_SUITE}" = "gvisor" ]; then
    echo "Running gVisor syscall tests..."
    if ! "${GVISOR_DIR}/run_gvisor_test.sh"; then
        echo "Error: gVisor syscall tests failed." >&2
        exit 2
    fi
elif [ "${CONFORMANCE_TEST_SUITE}" == "kselftest" ]; then
    echo "Running Linux kernel selftest..."
    if ! "${KSELFTEST_DIR}/run_kselftest.sh"; then
        echo "Error: Linux kernel selftest failed." >&2
        exit 3
    fi
elif [ "${CONFORMANCE_TEST_SUITE}" = "xfstests" ]; then
    echo "Running xfstests..."
    if [ -n "${XFSTESTS_RUNLIST}" ]; then
        set -- -R "${XFSTESTS_RUNLIST}"
    else
        set --
    fi
    if ! "${XFSTESTS_DIR}/run_xfstests.sh" "$@"; then
        echo "Error: xfstests failed." >&2
        exit 4
    fi
else
    echo "Error: Unknown test suite '${CONFORMANCE_TEST_SUITE}'." >&2
    exit 5
fi

echo "All conformance tests passed."
exit 0
