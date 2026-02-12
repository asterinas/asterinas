#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./epoll/epoll_err
./epoll/poll_err
./epoll/test_epoll_pwait.sh

./eventfd2/eventfd2

./file_io/access_err
./file_io/file_err
./file_io/iovec_err
