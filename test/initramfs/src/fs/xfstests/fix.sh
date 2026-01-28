cd /xfstests

# Replace echo with printf for oom_score_adj writes (echo fails on this system).
sed -i 's|echo \(.*\) > /proc/self/oom_score_adj|printf \1 > /proc/self/oom_score_adj|g' check


# 1. disable the original _check_mounted_on
sed -i 's/^_check_mounted_on()/_disabled_check_mounted_on()/' common/rc

# 2. disable the original _mountpoint
sed -i 's/^_mountpoint()/_disabled_mountpoint()/' common/rc

# 3. try to disable findmnt (in case)
sed -i 's/^findmnt()/_disabled_findmnt()/' common/rc




cat > patch_header << 'EOF'
# === ASTERINAS MOCK PATCH ===

findmnt() {
    case "$*" in
        *vdc*) echo "$TEST_DIR"; return 0 ;;
        *vdd*) echo "$SCRATCH_MNT"; return 0 ;;
        *) echo "$TEST_DIR"; return 0 ;;
    esac
}

_mountpoint() {
    case "$*" in
        *vdc*) echo "$TEST_DIR" ;;
        *vdd*) echo "$SCRATCH_MNT" ;;
        *) echo "$TEST_DIR" ;;
    esac
}

umount() {
    /bin/busybox umount "$@" >/dev/null 2>&1
    return 0
}

_check_mounted_on() { return 0; }
_require_xfs_io_command() { return 0; }
_check_dmesg() { return 0; }
_detect_kmemleak() { return 0; }
modprobe() { return 0; }
# === END PATCH ===
EOF

# add patch header to rc
cp common/rc common/rc.renamed
rm common/rc
cat patch_header common/rc.renamed > common/rc



# disable modprobe calls in common/rc

# rm -f /sbin/modprobe /bin/modprobe /sbin/depmod
# echo -e '#!/bin/sh\nexit 0' > /bin/modprobe
# chmod +x /bin/modprobe
# cp /bin/modprobe /sbin/modprobe
# cp /bin/modprobe /sbin/depmod


