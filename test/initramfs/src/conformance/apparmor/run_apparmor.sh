#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e
export PATH="@runtimePath@:/bin:/usr/bin"
mkdir -p /var/cache/apparmor
case "${CONFORMANCE_TEST_SELECTOR:-}" in
    "" | all)
        @bash@ /opt/apparmor/run-e2e.sh
        python3 /opt/apparmor/lifecycle.py
        exec python3 /opt/apparmor/admission.py
        ;;
    e2e | e2e.sh) exec @bash@ /opt/apparmor/run-e2e.sh ;;
    lifecycle) exec python3 /opt/apparmor/lifecycle.py ;;
    abi-v5) exec python3 /opt/apparmor/admission.py ;;
    *) echo "Error: unsupported AppArmor test selector: $CONFORMANCE_TEST_SELECTOR" >&2; exit 2 ;;
esac
