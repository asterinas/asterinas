# SPDX-License-Identifier: MPL-2.0

# Asterinas-specific compatibility overrides for upstream xfstests.
_df_device()
{
    if [ $# -ne 1 ]
    then
        echo "Usage: _df_device device" 1>&2
        exit 1
    fi

    local df_line mount_point

    df_line=$($DF_PROG 2>/dev/null | $AWK_PROG -v what=$1 '
        ($1==what) && (NF==1) {
            v=$1
            getline
            print v, $0
            exit
        }
        ($1==what) {
            print
            exit
        }
    ')
    if [ -n "$df_line" ]; then
        printf '%s\n' "$df_line"
        return 0
    fi

    mount_point=$(findmnt -rncv -S "$1" -o TARGET 2>/dev/null | head -n 1)
    if [ -n "$mount_point" ]; then
        $DF_PROG "$mount_point" 2>/dev/null | $AWK_PROG -v what="$mount_point" '
            NR == 2 && NF==1 {
                v=$1
                getline
                print v, $0
                exit 0
            }
            NR == 2 {
                print
                exit 0
            }
        '
    fi
}

_fs_type()
{
    if [ $# -ne 1 ]
    then
        echo "Usage: _fs_type device" 1>&2
        exit 1
    fi

    local df_type

    df_type=$(_df_device "$1" | $AWK_PROG '{ print $2 }' | \
        sed -e 's/nfs4/nfs/' -e 's/fuse.glusterfs/glusterfs/' \
            -e 's/fuse.ceph-fuse/ceph-fuse/')
    if [ -n "$df_type" ]; then
        printf '%s\n' "$df_type"
        return 0
    fi

    findmnt -rncv -S "$1" -o FSTYPE 2>/dev/null | head -n 1 | \
        sed -e 's/nfs4/nfs/' -e 's/fuse.glusterfs/glusterfs/' \
            -e 's/fuse.ceph-fuse/ceph-fuse/'
}

_check_if_dev_already_mounted()
{
    local dev=$1
    local mnt=$2
    local mount_rec

    mount_rec=$(findmnt -rncv -S "$dev" -o SOURCE,TARGET 2>/dev/null | \
        $AWK_PROG '!seen[$0]++ { print }' | head -n 1)
    [ -n "$mount_rec" ] || return 1

    if [ "$mount_rec" != "$dev $mnt" ]; then
        echo "dev=$dev is mounted but not on mnt=$mnt - aborting"
        echo "Already mounted result:"
        echo "$mount_rec"
        return 2
    fi
}
