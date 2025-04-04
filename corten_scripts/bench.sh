
#!/bin/bash

# args: bench.sh [linux|asterinas] [bench_output_file] [cmd_to_run_in_vm]
# envs: NR_CPUS CORTEN_RUN_ARGS

set -e

export QMP_PORT=3336

NR_CPUS=${NR_CPUS:-$(nproc --all)}

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
PIN_CPU_SCRIPT="$SCRIPT_DIR/pin_cpu.py"
TEST_RESULTS_DIR="$SCRIPT_DIR/../test_results"
TMUX_SESSION_NAME="bench_session"
mkdir -p "$TEST_RESULTS_DIR"

BENCH_TARGET=$1
BENCH_OUTPUT_FILE=$2
COMMAND_IN_VM=$3

if [ "$BENCH_TARGET" == "linux" ]; then
    START_VM_CMD="$SCRIPT_DIR/start_linux.sh $NR_CPUS"
    EXIT_COMMAND="; poweroff -f"
else
    START_VM_CMD="make run SMP=$NR_CPUS MEM=240G RELEASE_LTO=1 $CORTEN_RUN_ARGS"
    EXIT_COMMAND="; exit"
fi

pushd "$SCRIPT_DIR/.."

COMMAND_IN_VM+=$EXIT_COMMAND

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
echo "Running benchmark command: $COMMAND_IN_VM"
tmux select-window -t ${TMUX_SESSION_NAME}:0
tmux send-keys -t ${TMUX_SESSION_NAME}:0 "${COMMAND_IN_VM}" Enter

tmux attach -t ${TMUX_SESSION_NAME}:0

unset QMP_PORT

popd
