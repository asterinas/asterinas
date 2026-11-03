#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

if [ "$(uname -s)" != Linux ]; then
	echo "AppArmor fixtures can only be generated on Linux" >&2
	exit 1
fi

parser=${APPARMOR_PARSER:-apparmor_parser}
version=$($parser --version 2>&1 | sed -n 's/^AppArmor parser version //p' | head -n 1)
if [ "$version" != 5.0.2 ]; then
	echo "apparmor_parser 5.0.2 is required (found: ${version:-unknown})" >&2
	exit 1
fi

cd "$(dirname "$0")"

compile()
{
	features_file=$1
	profile=$2
	output=$3
	shift 3
	$parser --skip-kernel-load --skip-cache --stdout --quiet \
		--zstd-compress-level=0 --features-file "$features_file" \
		"$@" "$profile" > "$output"
}

compile features observe.profile observe.bin

check_fixture()
{
	file=$1
	size=$2
	hash=$3
	actual_size=$(wc -c < "$file" | tr -d ' ')
	actual_hash=$(sha256sum "$file" | cut -d ' ' -f 1)
	[ "$actual_size" = "$size" ] && [ "$actual_hash" = "$hash" ] || {
		echo "$file does not match its frozen size/hash" >&2
		exit 1
	}
}

check_fixture observe.bin 118 912a45cf505271eee29e01e01d6d41abbe32ee540f94a10886746c5874400c16
sha256sum observe.bin > SHA256SUMS
