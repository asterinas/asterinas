#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

set -eu

usage() {
    cat <<'EOF'
Usage: compile-policy.sh [-f FEATURES] PROFILE OUTPUT

Compiles a Linux AppArmor text profile into the binary policy stream accepted
by Asterinas securityfs AppArmor policy files.

Environment:
  APPARMOR_PARSER              apparmor_parser path or command name
  ASTERINAS_APPARMOR_FEATURES  feature ABI file passed to apparmor_parser -M
EOF
}

write_default_features() {
    cat > "$1" <<'EOF'
caps {mask {chown dac_override dac_read_search fowner fsetid kill setgid setuid setpcap linux_immutable net_bind_service net_broadcast net_admin net_raw ipc_lock ipc_owner sys_module sys_rawio sys_chroot sys_ptrace sys_pacct sys_admin sys_boot sys_nice sys_resource sys_time sys_tty_config mknod lease audit_write audit_control setfcap mac_override mac_admin syslog wake_alarm block_suspend audit_read perfmon bpf checkpoint_restore}}
file {mask {create read write exec append mmap_exec link}}
domain {change_profile yes, change_onexec yes, stack yes, version 1.2, fix_binfmt_elf_mmap yes, post_nnp_subset yes, computed_longest_left yes}
policy {set_load yes, versions {v7 yes, v6 yes, v5 yes}, permstable32 {allow deny subtree cond kill complain prompt audit quiet hide xindex tag label}, permstable32_version {0x000002}}
EOF
}

features=${ASTERINAS_APPARMOR_FEATURES:-}
while getopts "f:h" option; do
    case "$option" in
        f)
            features=$OPTARG
            ;;
        h)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done
shift $((OPTIND - 1))

if [ "$#" -ne 2 ]; then
    usage >&2
    exit 2
fi

profile=$1
output=$2
parser=${APPARMOR_PARSER:-apparmor_parser}

if ! command -v "$parser" >/dev/null 2>&1; then
    if [ -x /usr/sbin/apparmor_parser ]; then
        parser=/usr/sbin/apparmor_parser
    else
        echo "compile-policy.sh: apparmor_parser was not found" >&2
        exit 127
    fi
fi

tmp_features=
tmp_output=$(mktemp "${TMPDIR:-/tmp}/asterinas-apparmor-policy.XXXXXX")
cleanup() {
    if [ -n "$tmp_output" ]; then
        rm -f "$tmp_output"
    fi
    if [ -n "$tmp_features" ]; then
        rm -f "$tmp_features"
    fi
}
trap cleanup EXIT HUP INT TERM

if [ -z "$features" ]; then
    tmp_features=$(mktemp "${TMPDIR:-/tmp}/asterinas-apparmor-features.XXXXXX")
    write_default_features "$tmp_features"
    features=$tmp_features
fi

mkdir -p "$(dirname "$output")"
"$parser" -Q -q -S -M "$features" "$profile" > "$tmp_output"
mv "$tmp_output" "$output"
tmp_output=
