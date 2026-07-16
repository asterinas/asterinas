// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/epoll.h>
#include <sys/eventfd.h>
#include <sys/inotify.h>
#include <sys/ioctl.h>
#include <sys/signalfd.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/timerfd.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <unistd.h>

#include "../../common/test.h"

enum fd_index {
	/* Objects opened through filesystem paths. */
	REGULAR_FILE_FD,
	DIRECTORY_FD,
	NULL_DEVICE_FD,
	PROCFS_FILE_FD,

	/* Anonymous and special file descriptors. */
	EPOLL_FD,
	EVENT_FD,
	INOTIFY_FD,
	PID_FD,
	SIGNAL_FD,
	TIMER_FD,

	/* Inter-process communication file descriptors. */
	SOCKET_FD,
	PIPE_READ_FD,
	PIPE_WRITE_FD,
	FD_COUNT,
};

static int fds[FD_COUNT];

enum async_support {
	DOES_NOT_SUPPORT_ASYNC,
	SUPPORTS_ASYNC,
};

static const enum async_support async_support_by_fd[FD_COUNT] = {
	[REGULAR_FILE_FD] = DOES_NOT_SUPPORT_ASYNC,
	[DIRECTORY_FD] = DOES_NOT_SUPPORT_ASYNC,
	[NULL_DEVICE_FD] = DOES_NOT_SUPPORT_ASYNC,
	[PROCFS_FILE_FD] = DOES_NOT_SUPPORT_ASYNC,

	[EPOLL_FD] = DOES_NOT_SUPPORT_ASYNC,
	[EVENT_FD] = DOES_NOT_SUPPORT_ASYNC,
	[INOTIFY_FD] = SUPPORTS_ASYNC,
	[PID_FD] = DOES_NOT_SUPPORT_ASYNC,
	[SIGNAL_FD] = DOES_NOT_SUPPORT_ASYNC,
	[TIMER_FD] = DOES_NOT_SUPPORT_ASYNC,

	[SOCKET_FD] = SUPPORTS_ASYNC,
	[PIPE_READ_FD] = SUPPORTS_ASYNC,
	[PIPE_WRITE_FD] = SUPPORTS_ASYNC,
};

FN_SETUP(create)
{
	fds[REGULAR_FILE_FD] = CHECK(open("fcntl_status_flags_file",
					  O_CREAT | O_RDWR | O_TRUNC, 0600));
	fds[DIRECTORY_FD] = CHECK(open(".", O_RDONLY | O_DIRECTORY));
	fds[NULL_DEVICE_FD] = CHECK(open("/dev/null", O_RDWR));
	fds[PROCFS_FILE_FD] = CHECK(open("/proc/self/maps", O_RDONLY));

	fds[EPOLL_FD] = CHECK(epoll_create1(0));
	fds[EVENT_FD] = CHECK(eventfd(0, 0));
	fds[INOTIFY_FD] = CHECK(inotify_init1(0));
	fds[PID_FD] = CHECK(syscall(SYS_pidfd_open, getpid(), 0));

	sigset_t mask;
	CHECK(sigemptyset(&mask));
	CHECK(sigaddset(&mask, SIGUSR1));
	fds[SIGNAL_FD] = CHECK(signalfd(-1, &mask, 0));
	fds[TIMER_FD] = CHECK(timerfd_create(CLOCK_MONOTONIC, 0));

	fds[SOCKET_FD] = CHECK(socket(AF_UNIX, SOCK_STREAM, 0));
	CHECK(pipe(&fds[PIPE_READ_FD]));
}
END_SETUP()

static int set_flag_and_get_flags(int fd, int flag)
{
	int flags = fcntl(fd, F_GETFL, 0);
	if (flags < 0)
		return -1;
	if (fcntl(fd, F_SETFL, flags | flag) < 0)
		return -1;

	return fcntl(fd, F_GETFL, 0);
}

static int set_async_with_ioctl_and_get_flags(int fd, int is_async)
{
	if (ioctl(fd, FIOASYNC, &is_async) < 0)
		return -1;

	return fcntl(fd, F_GETFL, 0);
}

FN_TEST(set_nonblocking_on_file_types)
{
	for (size_t i = 0; i < FD_COUNT; i++)
		TEST_RES(set_flag_and_get_flags(fds[i], O_NONBLOCK),
			 (_ret & O_NONBLOCK) != 0);
}
END_TEST()

FN_TEST(set_append_on_file_types)
{
	for (size_t i = 0; i < FD_COUNT; i++)
		TEST_RES(set_flag_and_get_flags(fds[i], O_APPEND),
			 (_ret & O_APPEND) != 0);
}
END_TEST()

FN_TEST(set_noatime_on_file_types)
{
	/* Regression tests run as root, which may set `O_NOATIME` on any inode. */
	for (size_t i = 0; i < FD_COUNT; i++)
		TEST_RES(set_flag_and_get_flags(fds[i], O_NOATIME),
			 (_ret & O_NOATIME) != 0);
}
END_TEST()

FN_TEST(set_async_with_ioctl_on_file_types)
{
	for (size_t i = 0; i < FD_COUNT; i++) {
		if (async_support_by_fd[i] == SUPPORTS_ASYNC) {
			TEST_RES(set_async_with_ioctl_and_get_flags(fds[i], 1),
				 (_ret & O_ASYNC) != 0);
		} else {
			TEST_ERRNO(set_async_with_ioctl_and_get_flags(fds[i],
								      1),
				   ENOTTY);
		}
		TEST_RES(set_async_with_ioctl_and_get_flags(fds[i], 0),
			 (_ret & O_ASYNC) == 0);
	}
}
END_TEST()

FN_TEST(set_async_on_file_types)
{
	/*
	 * Linux keeps `O_ASYNC` only when the file type provides a fasync callback.
	 * `F_SETFL` still succeeds for unsupported file types but leaves the bit clear.
	 */
	for (size_t i = 0; i < FD_COUNT; i++)
		TEST_RES(set_flag_and_get_flags(fds[i], O_ASYNC),
			 ((_ret & O_ASYNC) != 0) ==
				 (async_support_by_fd[i] == SUPPORTS_ASYNC));
}
END_TEST()

FN_TEST(set_direct_on_file_types)
{
	TEST_RES(set_flag_and_get_flags(fds[REGULAR_FILE_FD], O_DIRECT),
		 (_ret & O_DIRECT) != 0);

	for (size_t i = DIRECTORY_FD; i < PIPE_READ_FD; i++) {
		TEST_ERRNO(set_flag_and_get_flags(fds[i], O_DIRECT), EINVAL);
	}

	/*
	 * FIXME: Linux enables pipe packet mode when `O_DIRECT` is set with `F_SETFL`,
	 * while Asterinas returns `EINVAL` because packet mode is not supported yet.
	 */
#ifdef __asterinas__
	for (size_t i = PIPE_READ_FD; i < FD_COUNT; i++)
		TEST_ERRNO(set_flag_and_get_flags(fds[i], O_DIRECT), EINVAL);
#else
	for (size_t i = PIPE_READ_FD; i < FD_COUNT; i++)
		TEST_RES(set_flag_and_get_flags(fds[i], O_DIRECT),
			 (_ret & O_DIRECT) != 0);
#endif
}
END_TEST()

FN_SETUP(cleanup)
{
	for (size_t i = 0; i < FD_COUNT; i++)
		CHECK(close(fds[i]));
	CHECK(unlink("fcntl_status_flags_file"));
}
END_SETUP()
