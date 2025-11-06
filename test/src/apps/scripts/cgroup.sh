#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

CGROUP_ROOT="/sys/fs/cgroup"
CGROUP_NAME="user"
PROCESS_ID=1

log_step() {
    echo -e "\n==> $1"
}

verify() {
    local description=$1
    local command=$2
    local expected=$3
    
    log_step "Verify: $description"
    echo "Run: $command"
    local result=$(eval "$command")
    echo "Got: $result"
    echo "Expected: $expected"
    
    if [ "$result" != "$expected" ]; then
        echo -e "Error: Verification failed!"
        exit 1
    else
        echo -e "Verified"
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

log_step "1. Change to cgroup root directory"
cd "$CGROUP_ROOT"
echo "Current directory: $(pwd)"

log_step "2. Create user hierarchy"
mkdir -p "$CGROUP_NAME"
verify "user hierarchy exists" "ls -d $CGROUP_NAME" "$CGROUP_NAME"

log_step "3. Enter user directory"
cd "$CGROUP_NAME"
echo "Current directory: $(pwd)"

log_step "4. Check initial cgroup of process 1"
verify "Process 1 initially in root cgroup" "grep -a '0::' /proc/$PROCESS_ID/cgroup" "0::/"

log_step "4a. Check initial populated status"
verify "Initial cgroup.events populated=0" "grep '^populated' cgroup.events" "populated 0"

log_step "5. Bind process 1 to user hierarchy"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 added to user hierarchy" "cat cgroup.procs | grep -w $PROCESS_ID" "$PROCESS_ID"

log_step "5a. Check populated status after binding"
verify "cgroup.events populated=1 after binding" "grep '^populated' cgroup.events" "populated 1"

log_step "6. Check process 1 cgroup again"
verify "Process 1 now in user hierarchy" "grep -a '0::' /proc/$PROCESS_ID/cgroup" "0::/$CGROUP_NAME"

log_step "7. Return to parent directory"
cd ..
echo "Current directory: $(pwd)"

log_step "8. Check parent cgroup.procs contents"
echo "cgroup.procs contents:"
cat cgroup.procs
verify "cgroup.procs now does not contain process 1" "cat cgroup.procs | grep -w $PROCESS_ID" ""

log_step "9. Move process 1 back to root cgroup"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 back in root cgroup" "grep -a '0::' /proc/$PROCESS_ID/cgroup" "0::/"

log_step "10. Remove user hierarchy"
rmdir "$CGROUP_NAME"
verify "user hierarchy removed" "ls -d $CGROUP_NAME 2>&1" "ls: $CGROUP_NAME: No such file or directory"

echo -e "\nAll test steps completed successfully!"