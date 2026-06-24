#!/bin/sh
set +e

LOG=${KATA_PULL_LOG:-/tmp/kata-image-pull.log}
RUN_LOG=${KATA_PULL_RUN_LOG:-/tmp/kata-image-pull-run.txt}
STEP_LOG=${KATA_PULL_STEP_LOG:-/tmp/kata-image-pull-step.out}
CID=${KATA_PULL_CID:-kata-pull-busybox}
IMAGE_LIST=${KATA_PULL_IMAGES:-docker.io/library/busybox:latest docker.m.daocloud.io/library/busybox:latest}
RUN_IMAGE_AFTER_PULL=${KATA_PULL_RUN_IMAGE:-1}

: > "$LOG"
: > "$RUN_LOG"
: > "$STEP_LOG"

say() {
    printf '%s\n' "$*" | tee -a "$LOG"
}

show_step_log() {
    cat "$STEP_LOG" 2>&1 | tee -a "$LOG"
    : > "$STEP_LOG"
}

say "=== proxy env ==="
env | grep -i '_proxy\|KATA_PULL_IMAGES\|KATA_PULL_RUN_IMAGE' | sort 2>&1 | tee -a "$LOG" || true
say ""
say "=== resolv.conf ==="
cat /etc/resolv.conf > "$STEP_LOG" 2>&1 || true
show_step_log
say ""
say "=== network state ==="
say "--- ip addr ---"
timeout 5s ip addr > "$STEP_LOG" 2>&1 || true
show_step_log
say "--- ip route ---"
timeout 5s ip route > "$STEP_LOG" 2>&1 || true
show_step_log
say ""
say "=== registry probes ==="
for url in \
    https://registry-1.docker.io/v2/ \
    https://docker.m.daocloud.io/v2/; do
    say "--- $url ---"
    timeout 30s curl -v -sSI --max-time 20 "$url" > "$STEP_LOG" 2>&1 || true
    head -80 "$STEP_LOG" 2>&1 | tee -a "$LOG"
    : > "$STEP_LOG"
done
say ""

pulled=
for image in $IMAGE_LIST; do
    say "=== pull $image ==="
    timeout 420s ctr --debug images pull "$image" > "$STEP_LOG" 2>&1
    rc=$?
    show_step_log
    say "pull_rc:$rc image:$image"
    if [ "$rc" -eq 0 ]; then
        pulled=$image
        break
    fi
done

if [ -z "$pulled" ]; then
    say "PULL_RESULT=failed"
    exit 1
fi

say "PULL_RESULT=ok image=$pulled"
ctr images ls > "$STEP_LOG" 2>&1 || true
show_step_log

if [ "$RUN_IMAGE_AFTER_PULL" != "1" ]; then
    say "RUN_RESULT=skipped"
    exit 0
fi

say "=== cleanup prior container ==="
ctr tasks kill "$CID" > "$STEP_LOG" 2>&1 || true
show_step_log
ctr containers rm "$CID" > "$STEP_LOG" 2>&1 || true
show_step_log

say "=== run $pulled with kata ==="
timeout 900s ctr --debug run \
    --runtime io.containerd.kata.v2 \
    "$pulled" "$CID" \
    /bin/sh -c 'echo HI-PULL; exit 7' > "$RUN_LOG" 2>&1
run_rc=$?
echo "exit:$run_rc" | tee -a "$RUN_LOG"
cat "$RUN_LOG" | tee -a "$LOG"

exit 0
