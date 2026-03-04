#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

CGROUP_ROOT="/sys/fs/cgroup"
CGROUP_NAME="user"
PROCESS_ID=1

# --- Helpers ------------------------------------------------------------------

log_section() {
    echo ""
    echo "=== $1 ==="
}

log_step() {
    echo ""
    echo "==> $1"
}

verify() {
    local description=$1
    local command=$2
    local expected=$3

    log_step "Verify: $description"
    echo "Run: $command"
    local result=$(eval "$command" 2>&1)
    echo "Got: $result"
    echo "Expected: $expected"

    if [ "$result" != "$expected" ]; then
        echo "Error: Verification failed!"
        exit 1
    else
        echo "Verified"
    fi
}

verify_ge() {
    local description=$1
    local command=$2
    local min=$3

    log_step "Verify: $description"
    echo "Run: $command"
    local result=$(eval "$command" 2>&1)
    echo "Got: $result"
    echo "Expected: >= $min"

    if [ "$result" -lt "$min" ] 2>/dev/null; then
        echo "Error: Verification failed!"
        exit 1
    else
        echo "Verified"
    fi
}

cleanup() {
    log_step "Cleaning up"

    if [ -f "$CGROUP_ROOT/cgroup.procs" ]; then
        echo "Moving process $PROCESS_ID back to root cgroup"
        echo $PROCESS_ID > "$CGROUP_ROOT/cgroup.procs" 2>/dev/null || true
    fi

    if [ -d "$CGROUP_ROOT/$CGROUP_NAME" ]; then
        echo "Removing test cgroup: $CGROUP_ROOT/$CGROUP_NAME"
        rmdir "$CGROUP_ROOT/$CGROUP_NAME" 2>/dev/null || true
    fi

    echo "Cleanup complete"
}

trap cleanup EXIT

# --- Section 1: Basic cgroup setup --------------------------------------------

log_section "Section 1: Basic cgroup setup"

log_step "1.1 Change to cgroup root directory"
cd "$CGROUP_ROOT"
echo "Current directory: $(pwd)"

log_step "1.2 Check initial controller attributes"
verify "cpuset.cpus.effective exists in root" \
    "ls cpuset.cpus.effective" \
    "cpuset.cpus.effective"

log_step "1.3 Create user hierarchy"
mkdir -p "$CGROUP_NAME"
verify "user hierarchy exists" \
    "ls -d $CGROUP_NAME" \
    "$CGROUP_NAME"

log_step "1.4 Enter user directory"
cd "$CGROUP_NAME"
echo "Current directory: $(pwd)"

log_step "1.5 Check initial memory.max in child"
verify "memory.max doesn't exist initially" \
    "ls memory.max" \
    "ls: memory.max: No such file or directory"

log_step "1.6 Check initial cgroup of process 1"
verify "Process 1 initially in root cgroup" \
    "grep -a '0::/' /proc/$PROCESS_ID/cgroup" \
    "0::/"

log_step "1.7 Check initial cgroup.events populated status"
verify "Initial cgroup.events populated=0" \
    "grep '^populated' cgroup.events" \
    "populated 0"

# --- Section 2: Controller activation / deactivation -------------------------

log_section "Section 2: Controller activation / deactivation"

log_step "2.1 Enable memory and pids in root"
cd "$CGROUP_ROOT"
echo "+memory +pids" > cgroup.subtree_control
verify "root subtree_control has memory and pids" \
    "cat cgroup.subtree_control" \
    "memory pids"

log_step "2.2 Check child now has memory.max and pids.max"
cd "$CGROUP_NAME"
echo "Current directory: $(pwd)"
verify "memory.max now exists" \
    "ls memory.max" \
    "memory.max"
verify "pids.max now exists" \
    "ls pids.max" \
    "pids.max"

log_step "2.3 Check child cgroup.controllers"
verify "child controllers are memory and pids" \
    "cat cgroup.controllers" \
    "memory pids"

log_step "2.4 Disable memory in root"
cd "$CGROUP_ROOT"
echo "-memory" > cgroup.subtree_control
verify "root subtree_control removed memory" \
    "cat cgroup.subtree_control" \
    "pids"

log_step "2.5 Check child lost memory.max after deactivation"
cd "$CGROUP_NAME"
verify "memory.max removed after disabling" \
    "ls memory.max" \
    "ls: memory.max: No such file or directory"

# --- Section 3: Process membership --------------------------------------------

log_section "Section 3: Process membership"

log_step "3.1 Bind process 1 to user hierarchy"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 added to user hierarchy" \
    "cat cgroup.procs | grep -w $PROCESS_ID" \
    "$PROCESS_ID"

log_step "3.2 Try enabling pids in child with process attached (expect EBUSY)"
verify "Cannot enable pids with process attached" \
    "echo +pids > cgroup.subtree_control" \
    "sh: write error: Device or resource busy"

log_step "3.3 Remove process 1 from child"
cd "$CGROUP_ROOT"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 back in root cgroup" \
    "grep -a '0::/' /proc/$PROCESS_ID/cgroup" \
    "0::/"

log_step "3.4 Enable pids in child after process removed (expect success)"
cd "$CGROUP_NAME"
echo "+pids" > cgroup.subtree_control
verify "pids enabled successfully in child" \
    "cat cgroup.subtree_control" \
    "pids"

log_step "3.5 Try binding process when child pids enabled (expect EBUSY)"
verify "Cannot bind process when child pids enabled" \
    "echo $PROCESS_ID > cgroup.procs" \
    "sh: write error: Device or resource busy"

log_step "3.6 Disable pids in child and bind process again"
echo "-pids" > cgroup.subtree_control
echo $PROCESS_ID > cgroup.procs
verify "Process 1 added to user hierarchy" \
    "cat cgroup.procs | grep -w $PROCESS_ID" \
    "$PROCESS_ID"

# --- Section 4: pids controller -----------------------------------------------

log_section "Section 4: pids controller"

# -- 4.1 pids.max --------------------------------------------------------------

log_section "Section 4.1: pids.max"

log_step "4.1.1 Check default pids.max is 'max' (unlimited)"
verify "pids.max defaults to max" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.max" \
    "max"

log_step "4.1.2 Set pids.max to a specific limit"
echo 10 > "$CGROUP_ROOT/$CGROUP_NAME/pids.max"
verify "pids.max set to 10" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.max" \
    "10"

log_step "4.1.3 Reset pids.max back to unlimited"
echo "max" > "$CGROUP_ROOT/$CGROUP_NAME/pids.max"
verify "pids.max reset to max" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.max" \
    "max"

log_step "4.1.4 Verify pids.max enforces the fork limit"

# Write a helper script that will run inside the cgroup.
# It moves itself into the cgroup, then attempts a fork.
# The result (success or failure) is written to a temp file.
# This helper runs as a completely separate process, so when the
# fork inside it fails and the shell aborts, our main script is
# unaffected.
RESULT_FILE=$(mktemp)
HELPER_SCRIPT=$(mktemp)

cat > "$HELPER_SCRIPT" << 'EOF'
#!/bin/sh
CGROUP_PATH="$1"
RESULT_FILE="$2"

# Join the cgroup.
echo $$ > "$CGROUP_PATH/cgroup.procs"

# Read the current pid count (built-in read, no fork).
read CURRENT_IN_CGROUP < "$CGROUP_PATH/pids.current"

# Set pids.max to exactly the current count so no further fork is allowed.
echo $CURRENT_IN_CGROUP > "$CGROUP_PATH/pids.max"

# Attempt to fork. If the kernel rejects it, this shell will abort.
# We write the result before attempting so we have a baseline.
echo "fork_failed" > "$RESULT_FILE"

# This fork is the one that should be rejected.
sh -c "echo fork_succeeded > $RESULT_FILE" 2>/dev/null
EOF

chmod +x "$HELPER_SCRIPT"

# Run the helper as a separate process. If it crashes due to fork failure,
# only the helper exits; our main script continues normally.
sh "$HELPER_SCRIPT" "$CGROUP_ROOT/$CGROUP_NAME" "$RESULT_FILE" 2>/dev/null || true

# The helper has exited (either normally or due to fork failure).
# Restore pids.max before reading results.
echo "max" > "$CGROUP_ROOT/$CGROUP_NAME/pids.max"

FORK_RESULT=$(cat "$RESULT_FILE")
rm -f "$RESULT_FILE" "$HELPER_SCRIPT"

echo "Fork attempt result: $FORK_RESULT"

if [ "$FORK_RESULT" = "fork_failed" ]; then
    echo "Verified: fork was correctly rejected by pids.max"
else
    echo "Error: fork succeeded despite pids.max limit, got: $FORK_RESULT"
    exit 1
fi

# -- 4.2 pids.current ----------------------------------------------------------

log_section "Section 4.2: pids.current"

log_step "4.2.1 Check pids.current with process 1 in cgroup"
verify_ge "pids.current >= 1 with one process" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.current" \
    1

log_step "4.2.2 Spawn a child process and verify pids.current increases"

# Read baseline before the shell joins the cgroup so the cat itself is safe.
BEFORE=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.current")
echo "pids.current before fork: $BEFORE"

# Move the current shell into the user cgroup.
echo $$ > "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs"

# Fork the child process that we want to observe.
sleep 100 &
CHILD_PID=$!

# Use the shell built-in read to avoid an extra fork for the cat.
read AFTER < "$CGROUP_ROOT/$CGROUP_NAME/pids.current"
echo "pids.current after fork: $AFTER"

# Move the shell back to the root cgroup; subsequent commands are safe.
echo $$ > "$CGROUP_ROOT/cgroup.procs"

# Verify the count increased (sleep is still running in the background).
verify_ge "pids.current increased after fork" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.current" \
    $((BEFORE + 1))

log_step "4.2.3 Kill child process and verify pids.current decreases"

# Kill the child and wait for it to fully exit.
kill $CHILD_PID 2>/dev/null || true
wait $CHILD_PID 2>/dev/null || true

# Give the cgroup accounting a moment to update.
sleep 0.2

# pids.current should be back to the baseline (shell is no longer in cgroup).
verify "pids.current back to pre-fork value" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.current" \
    "$BEFORE"

log_step "4.2.4 Verify pids.current does not exceed pids.max"
echo 5 > "$CGROUP_ROOT/$CGROUP_NAME/pids.max"
CURRENT=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.current")
MAX=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.max")
log_step "pids.current=$CURRENT, pids.max=$MAX"
if [ "$CURRENT" -gt "$MAX" ]; then
    echo "Error: pids.current ($CURRENT) exceeds pids.max ($MAX)!"
    exit 1
else
    echo "Verified: pids.current within pids.max"
fi
echo "max" > "$CGROUP_ROOT/$CGROUP_NAME/pids.max"

log_step "4.2.5 Verify pids.current accuracy after pids controller is toggled"
# This test checks that disabling and re-enabling the pids controller does not
# corrupt the pid accounting: pids.current must still reflect the actual number
# of processes present in the cgroup.

# Spawn two long-lived background processes and move them into the cgroup.
sleep 100 &
BG1=$!
sleep 100 &
BG2=$!
echo $BG1 > "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs"
echo $BG2 > "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs"

# Count how many PIDs the cgroup actually contains right now.
ACTUAL_BEFORE=$(wc -l < "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs")
echo "Processes in cgroup before toggle: $ACTUAL_BEFORE"
echo "pids.current before toggle: $(cat $CGROUP_ROOT/$CGROUP_NAME/pids.current)"

# Disable the pids controller at the root level.
echo "-pids" > "$CGROUP_ROOT/cgroup.subtree_control"
echo "pids controller disabled"

# Re-enable the pids controller at the root level.
echo "+pids" > "$CGROUP_ROOT/cgroup.subtree_control"
echo "pids controller re-enabled"

# After the toggle, pids.current must equal the number of processes still
# present in cgroup.procs.
ACTUAL_AFTER=$(wc -l < "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs")
REPORTED_AFTER=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.current")
echo "Processes in cgroup after toggle: $ACTUAL_AFTER"
echo "pids.current after toggle: $REPORTED_AFTER"

# Clean up the background processes before asserting, so a failure does not
# leave them running.
kill $BG1 $BG2 2>/dev/null || true
wait $BG1 $BG2 2>/dev/null || true
sleep 0.2

# Now verify that pids.current matches the actual process count (both should
# have dropped to zero since we just killed the background jobs).
ACTUAL_FINAL=$(wc -l < "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs")
REPORTED_FINAL=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.current")
echo "Processes in cgroup after kill: $ACTUAL_FINAL"
echo "pids.current after kill: $REPORTED_FINAL"

if [ "$REPORTED_FINAL" -ne "$ACTUAL_FINAL" ]; then
    echo "Error: pids.current ($REPORTED_FINAL) does not match actual process count ($ACTUAL_FINAL) after controller toggle!"
    exit 1
else
    echo "Verified: pids.current matches actual process count after controller toggle"
fi

# -- 4.3 pids.peak -------------------------------------------------------------

log_section "Section 4.3: pids.peak"

log_step "4.3.1 Record initial pids.peak"
INITIAL_PEAK=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.peak")
echo "Initial pids.peak: $INITIAL_PEAK"
verify_ge "pids.peak >= 1 initially" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.peak" \
    1

log_step "4.3.2 Spawn multiple child processes to drive pids up"
echo $$ > "$CGROUP_ROOT/$CGROUP_NAME/cgroup.procs"

sleep 100 & PID1=$!
sleep 100 & PID2=$!
sleep 100 & PID3=$!

read PEAK_DURING < "$CGROUP_ROOT/$CGROUP_NAME/pids.peak"

echo $$ > "$CGROUP_ROOT/cgroup.procs"

echo "pids.peak during burst: $PEAK_DURING"
verify_ge "pids.peak increased during burst" \
    "cat $CGROUP_ROOT/$CGROUP_NAME/pids.peak" \
    $((INITIAL_PEAK + 1))

log_step "4.3.3 Kill child processes"
kill $PID1 $PID2 $PID3 2>/dev/null || true
wait $PID1 $PID2 $PID3 2>/dev/null || true

log_step "4.3.4 Verify pids.peak is retained after processes exit"

PEAK_AFTER=$(cat "$CGROUP_ROOT/$CGROUP_NAME/pids.peak")
echo "pids.peak after children exit: $PEAK_AFTER"

if [ "$PEAK_AFTER" -lt "$PEAK_DURING" ]; then
    echo -e "Error: pids.peak decreased after exit (should be monotonically non-decreasing)!"
    exit 1
else
    echo -e "Verified: pids.peak retained"
fi

# ─── Section 5: Teardown ──────────────────────────────────────────────────────

log_section "Section 5: Teardown"

log_step "5.1 Move process 1 back to root"
cd "$CGROUP_ROOT"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 back in root cgroup" \
    "grep -a '0::' /proc/$PROCESS_ID/cgroup" \
    "0::/"

log_step "5.2 Remove user hierarchy"
rmdir "$CGROUP_NAME"
verify "user hierarchy removed" \
    "ls -d $CGROUP_NAME" \
    "ls: $CGROUP_NAME: No such file or directory"

echo -e "All test steps completed successfully!"
