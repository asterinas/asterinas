#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

# Host preparation and target checks execute the same numbered test separately.
_erofs_phase_init()
{
	[ "$#" -eq 2 ] || _fail "invalid phase arguments"
	case "$1" in
	--prepare) _erofs_phase=prepare ;;
	--check) _erofs_phase=check ;;
	*) _fail "invalid phase" ;;
	esac
	materials=$2
	set -e
	trap '_erofs_phase_exit $?' EXIT
	trap 'exit 1' HUP INT QUIT TERM
	_scratch_unmount || _fail "failed to clean scratch mount"
	_scratch2_unmount || _fail "failed to clean scratch2 mount"
	if _erofs_is_prepare; then
		mkdir -p "$materials"
	else
		cp "$materials/prepare.full" "$seqres.full" || _fail "missing preparation log"
	fi
}

_erofs_is_prepare()
{
	[ "$_erofs_phase" = prepare ]
}

_erofs_prepare_done()
{
	cp "$SCRATCH_DEV" "$materials/image" || _fail "failed to save image"
	status=0
	exit 0
}

_erofs_load_image()
{
	[ -b "$SCRATCH_DEV" ] || _fail "scratch block device missing"
	dd if="$materials/image" of="$SCRATCH_DEV" bs=1M >>"$seqres.full" 2>&1 ||
		_fail "failed to load image"
}

_erofs_collect_xattrs()
{
	local entry value
	while IFS= read -r entry; do
		printf '%s\n' "$entry" || return 1
		getfattr --absolute-names -d "$1/$entry" >"$tmp/xattr" || return 1
		value=$(tail -n+2 "$tmp/xattr") || return 1
		printf '%s\n' "$value" || return 1
	done <"$materials/files"
}

_erofs_phase_exit()
{
	local result=$1 clean_result=0
	trap - EXIT
	set +e
	if ! _erofs_is_prepare; then
		_check_results || result=1
	fi
	_scratch_unmount >>"$seqres.full" 2>&1 || clean_result=1
	_scratch2_unmount >>"$seqres.full" 2>&1 || clean_result=1
	# Never remove a directory through a mount whose unmount failed.
	if [ "$clean_result" -eq 0 ]; then
		cleanup >>"$seqres.full" 2>&1 || clean_result=1
	fi
	if [ "$clean_result" -eq 0 ]; then
		rm -rf "$tmp/" || clean_result=1
	fi
	[ "$clean_result" -eq 0 ] || result=1
	exit "$result"
}
