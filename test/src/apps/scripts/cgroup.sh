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
    local result=$(eval "$command" 2>&1)
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

log_step "1a. Check initial controller attributes"
verify "cpuset.cpus.effective exists in root" "ls cpuset.cpus.effective" "cpuset.cpus.effective"

log_step "2. Create user hierarchy"
mkdir -p "$CGROUP_NAME"
verify "user hierarchy exists" "ls -d $CGROUP_NAME" "$CGROUP_NAME"

log_step "3. Enter user directory"
cd "$CGROUP_NAME"
echo "Current directory: $(pwd)"

log_step "3a. Check initial memory.max in child"
verify "memory.max doesn't exist initially" "ls memory.max" "ls: memory.max: No such file or directory"

log_step "4. Check initial cgroup of process 1"
verify "Process 1 initially in root cgroup" "grep -a '0::' /proc/$PROCESS_ID/cgroup" "0::/"

log_step "4a. Check initial populated status"
verify "Initial cgroup.events populated=0" "grep '^populated' cgroup.events" "populated 0"

log_step "5. Enable memory and pids in root"
cd ..
echo "+memory +pids" > cgroup.subtree_control
verify "root subtree_control has memory and pids" "cat cgroup.subtree_control" "memory pids"

log_step "5a. Check child now has memory.max and pids.max"
cd "$CGROUP_NAME"
echo "Current directory: $(pwd)"
echo "Current directory list: $(ls)"
verify "memory.max now exists" "ls memory.max" "memory.max"
verify "pids.max now exists" "ls pids.max" "pids.max"

log_step "5b. Check child cgroup.controllers"
verify "child controllers are memory and pids" "cat cgroup.controllers" "memory pids"

log_step "6. Disable memory in root"
cd ..
echo "-memory" > cgroup.subtree_control
verify "root subtree_control removed memory" "cat cgroup.subtree_control" "pids"

log_step "6a. Check child lost memory.max"
cd "$CGROUP_NAME"
verify "memory.max removed after disabling" "ls memory.max" "ls: memory.max: No such file or directory"

log_step "7. Bind process 1 to user hierarchy"
echo $PROCESS_ID > cgroup.procs
verify "Process 1 added to user hierarchy" "cat cgroup.procs | grep -w $PROCESS_ID" "$PROCESS_ID"

log_step "7a. Try enabling pids in child (should fail with EBUSY)"
verify "Cannot enable pids with process attached" "echo +pids > cgroup.subtree_control" "sh: write error: Device or resource busy"

log_step "8. Remove process 1 from child"
cd ..
echo $PROCESS_ID > cgroup.procs
verify "Process 1 back in root cgroup" "grep -a '0::' /proc/$PROCESS_ID/cgroup" "0::/"

log_step "8a. Now enable pids in child (should succeed)"
cd "$CGROUP_NAME"
echo "+pids" > cgroup.subtree_control
verify "pids enabled successfully" "cat cgroup.subtree_control" "pids"

log_step "8b. Verify subtree_control is pids"
verify "subtree_control is pids" "cat cgroup.subtree_control" "pids"

log_step "9. Try binding process 1 again (should fail)"
verify "Cannot bind process when pids enabled" "echo $PROCESS_ID > cgroup.procs" "sh: write error: Device or resource busy"

log_step "10. Return to parent directory"
cd ..
echo "Current directory: $(pwd)"

log_step "11. Remove user hierarchy"
rmdir "$CGROUP_NAME"
verify "user hierarchy removed" "ls -d $CGROUP_NAME" "ls: $CGROUP_NAME: No such file or directory"

echo -e "\nAll test steps completed successfully!"