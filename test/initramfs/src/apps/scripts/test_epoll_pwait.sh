#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

EPOLLTEST_DIR=/test/epoll
cd ${EPOLLTEST_DIR}

echo "Start epoll_pwait test......"

# Step 2: Run epoll_pwait in the background
./epoll_pwait &
EPOLL_PID=$!

echo "epoll_pwait PID: $EPOLL_PID"

# Step 3: Wait for 1 seconds to let epoll_pwait initialize and block SIGUSR1
sleep 1

# Step 4: Send SIGUSR1 to epoll_pwait
kill -USR1 $EPOLL_PID
echo "Sent SIGUSR1 to PID $EPOLL_PID"

# Optional: Wait a bit more to see the output if the process is still running
sleep 3

# You can also wait till the subprocess epoll_pwait completely finishes
# wait $EPOLL_PID

echo "Test completed."
