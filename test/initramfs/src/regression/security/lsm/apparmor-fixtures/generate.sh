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
compile features demo.profile demo.bin
compile features wildcard.profile wildcard.bin

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
check_fixture demo.bin 1484 f877adf70efa115213a45d16cc11ff23d48b33ef5dee337c34ba2ee9008fa3a2
check_fixture wildcard.bin 1500 629983cf2684f71373c12cb92f9f8bf32537c44ebb195a164bb1b65b857c7f62

sha256sum observe.bin demo.bin wildcard.bin > SHA256SUMS
