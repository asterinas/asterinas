#/usr/bin/env sh

# SPDX-License-Identifier: MPL-2.0

set -euxo pipefail

(cat /proc/cpuinfo | grep -i "Intel") || (echo "Unsupported vendor for TDX. Expected Intel." && exit 1)

(dmesg | grep "TDX module initialized") || (echo "TDX is not initialized." && exit 1)

(rdmsr 0xfe --bitfield 15:15) > /tmp/rdmsr_results
rdmsr_results=$(cat /tmp/rdmsr-results)
if [ "$rdmsr_results" -ne 1 ]; then
	echo "Platform does not support TDX-SEAM"
	exit 1
fi

(cat /proc/cpuinfo | grep "TDX_host_platform") || (echo "TDX is not supported for this CPU." && exit 1)


