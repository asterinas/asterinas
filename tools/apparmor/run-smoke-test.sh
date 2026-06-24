#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

set -eu

usage() {
    cat <<'EOF'
Usage: run-smoke-test.sh POLICY_BIN [PROFILE_NAME]

Runs an AppArmor policy load and file-read enforcement smoke test inside an
Asterinas guest booted with security=apparmor or lsm=capability,apparmor,yama.

Compile POLICY_BIN on the host with tools/apparmor/compile-policy.sh before
copying it into the guest.
EOF
}

fail() {
    echo "apparmor smoke: $*" >&2
    exit 1
}

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    usage >&2
    exit 2
fi

policy_bin=$1
profile_name=${2:-asterinas-aa-smoke}
securityfs_root=${SECURITYFS_ROOT:-/sys/kernel/security}
apparmor_dir=$securityfs_root/apparmor
current_file=/proc/sys/kernel/apparmor/current
allowed_path=/tmp/asterinas-aa-allowed
denied_path=/tmp/asterinas-aa-denied
attr_exec=/proc/self/attr/exec
probe_busybox=/test/apparmor-busybox

[ -r "$policy_bin" ] || fail "policy binary is not readable: $policy_bin"
[ -x "$probe_busybox" ] || fail "static AppArmor probe busybox is missing: $probe_busybox"
[ -e /proc/self/attr/current ] || fail "/proc/self/attr/current is missing"
[ -e /proc/self/attr/exec ] || fail "/proc/self/attr/exec is missing"
[ -e /proc/self/attr/prev ] || fail "/proc/self/attr/prev is missing"
[ -e "$current_file" ] || fail "AppArmor current profile control file is missing"

current_profile=$(cat /proc/self/attr/current)
[ -n "$current_profile" ] || fail "current AppArmor profile is empty"

if [ ! -d "$apparmor_dir" ]; then
    mkdir -p "$securityfs_root"
    mount -t securityfs none "$securityfs_root" 2>/dev/null || true
fi

[ -d "$apparmor_dir" ] || fail "securityfs AppArmor directory is missing"
[ -w "$apparmor_dir/.replace" ] || fail "securityfs AppArmor .replace is not writable"
[ -r "$apparmor_dir/profiles" ] || fail "securityfs AppArmor profiles file is missing"

printf 'allowed\n' > "$allowed_path"
printf 'denied\n' > "$denied_path"

if ! cat "$policy_bin" > "$apparmor_dir/.replace"; then
    fail "policy load failed"
fi
grep -q "^$profile_name " "$apparmor_dir/profiles" \
    || fail "loaded profile is not listed: $profile_name"

printf '%s' "$profile_name" > "$attr_exec"
IFS= read -r onexec_profile < "$attr_exec" \
    || fail "AppArmor on-exec profile is not readable"
[ "$onexec_profile" = "$profile_name" ] \
    || fail "expected on-exec profile $profile_name, got $onexec_profile"

set +e
"$probe_busybox" sh -c '
profile_name=$1
allowed_path=$2
denied_path=$3

IFS= read -r current_profile < /proc/self/attr/current || exit 10
[ "$current_profile" = "$profile_name" ] || exit 11

IFS= read -r previous_profile < /proc/self/attr/prev || exit 12
[ "$previous_profile" = unconfined ] || exit 13

: < "$allowed_path" || exit 14
if ( : < "$denied_path" ) 2>/dev/null; then
    exit 15
fi
' sh "$profile_name" "$allowed_path" "$denied_path"
child_status=$?
set -e

[ "$child_status" -eq 0 ] \
    || fail "on-exec profile transition failed with status $child_status"

printf '\n' > "$attr_exec"
onexec_profile=$(cat "$attr_exec")
[ -z "$onexec_profile" ] \
    || fail "expected cleared on-exec profile, got $onexec_profile"

printf '%s' "$profile_name" > "$current_file"
IFS= read -r current_profile < /proc/self/attr/current \
    || fail "current AppArmor profile is not readable after confinement"
[ "$current_profile" = "$profile_name" ] \
    || fail "expected current profile $profile_name, got $current_profile"

: < "$allowed_path" || fail "allowed file read was denied"

set +e
(: < "$denied_path") 2>/dev/null
denied_status=$?
set -e

if [ "$denied_status" -eq 0 ]; then
    fail "denied file read unexpectedly succeeded"
fi

echo "apparmor smoke: passed"
