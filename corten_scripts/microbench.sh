#!/bin/bash

# args: microbench.sh [linux/asterinas]

set -e

export QMP_PORT=3336

NR_CPUS=384

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
PIN_CPU_SCRIPT="$SCRIPT_DIR/pin_cpu.py"
TEST_RESULTS_DIR="$SCRIPT_DIR/../test_results"
TMUX_SESSION_NAME="microbench_session"
mkdir -p "$TEST_RESULTS_DIR"

BENCH_TARGET=$1
if [ "$BENCH_TARGET" == "linux" ]; then
    START_VM_CMD="$SCRIPT_DIR/start_linux.sh $NR_CPUS"
    BENCH_OUTPUT_FILE="$TEST_RESULTS_DIR/linux_output.txt"
    EXIT_COMMAND="; poweroff -f"
else
    START_VM_CMD="make run SMP=$NR_CPUS MEM=240G RELEASE_LTO=1"
    BENCH_OUTPUT_FILE="$TEST_RESULTS_DIR/aster_output.txt"
    EXIT_COMMAND="; exit"
fi

pushd "$SCRIPT_DIR/.."

run_microbench() {
    # Usage: run_microbench <command>
    COMMAND=$1
    COMMAND+=$EXIT_COMMAND

    tmux new-session -d -s ${TMUX_SESSION_NAME}

    ASTER_SESSION_KEYS=$START_VM_CMD
    ASTER_SESSION_KEYS+=" 2>&1 | tee -a ${BENCH_OUTPUT_FILE}"
    # Exit session when the command finishes
    ASTER_SESSION_KEYS+="; exit"

    echo "Starting VM in tmux session ${TMUX_SESSION_NAME}:0 with command:"
    echo "# $ASTER_SESSION_KEYS"
    tmux send-keys -t ${TMUX_SESSION_NAME}:0 "$ASTER_SESSION_KEYS" Enter

    echo "Wait for \"~ \#\" shell prompt to appear in $BENCH_OUTPUT_FILE"
    while ! tail -n 1 $BENCH_OUTPUT_FILE | grep -q "~ #"; do
        echo "Waiting..."
        sleep 5
    done

    # Bind cores
    echo "Binding cores to VM"
    python3 $PIN_CPU_SCRIPT $QMP_PORT $NR_CPUS

    # Run the microbenchmark
    echo "Running microbenchmark command: $COMMAND"
    tmux select-window -t ${TMUX_SESSION_NAME}:0
    tmux send-keys -t ${TMUX_SESSION_NAME}:0 "${COMMAND}" Enter

    tmux attach -t ${TMUX_SESSION_NAME}:0
}

if [ -f "$BENCH_OUTPUT_FILE" ]; then
    rm "$BENCH_OUTPUT_FILE"
fi

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    run_microbench "/test/scale/mmap unfixed $THREAD_COUNT"
    run_microbench "/test/scale/mmap fixed 1 $THREAD_COUNT"
    run_microbench "/test/scale/mmap_pf unfixed $THREAD_COUNT"
    run_microbench "/test/scale/mmap_pf fixed 1 $THREAD_COUNT"
    run_microbench "/test/scale/pf 0 $THREAD_COUNT"
    run_microbench "/test/scale/pf 1 $THREAD_COUNT"
    run_microbench "/test/scale/munmap_virt 0 $THREAD_COUNT"
    run_microbench "/test/scale/munmap_virt 1 $THREAD_COUNT"
    run_microbench "/test/scale/munmap 0 $THREAD_COUNT"
    run_microbench "/test/scale/munmap 1 $THREAD_COUNT"
done

unset QMP_PORT

popd
