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
compile features label-a.profile label-a.bin
compile features label-b.profile label-b.bin
compile features label-script.profile label-script.bin
compile features label-broken.profile label-broken.bin
compile features label-deleted.profile label-deleted.bin
compile features label-chroot.profile label-chroot.bin
compile features demo1.profile demo1.bin
compile features demo2.profile demo2.bin

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
check_fixture label-a.bin 1476 8497d217b87d50456dc7ef1a93ba62622959d2b386d497fc5a8144c39e060cfb
check_fixture label-b.bin 1476 32de72f5802a4c5fd8b87bf3e982d10c715613a81ca609945f2172aae977a7a6
check_fixture label-script.bin 1540 d2278c25c2229270300750000849aff5ed0e451c9d27d49a94c43beb414795f5
check_fixture label-broken.bin 1556 1f0365684a2694132bd6aad7b9438a0a28c4b633b99b9910011f7f79e99c0584
check_fixture label-deleted.bin 1580 4f1e9d7f4276b624e32fe2e98292a26fef743439bafc06d455ae12113f659569
check_fixture label-chroot.bin 1612 b0a35a13f73bb21581c744d22592f34b7c6ae7ae6ac43903fa5b43af9a32816a
check_fixture demo1.bin 1508 e1e34831442a9dffbbcfb3a52a0b59f67c89fe42fcd82a636161a41aac86ab38
check_fixture demo2.bin 1508 36fc3bde7d94c1bfe655f35f29cb581ea00adbe20c30ea898abee426e93d4941

sha256sum observe.bin demo.bin wildcard.bin label-a.bin label-b.bin label-script.bin label-broken.bin label-deleted.bin label-chroot.bin demo1.bin demo2.bin > SHA256SUMS
