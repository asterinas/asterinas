// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <fcntl.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/eventfd.h>
#include <sys/timerfd.h>
#include <sys/signalfd.h>
#include <sys/epoll.h>
#include <sys/inotify.h>
#include <sys/wait.h>
#include <sys/syscall.h>
#include <sys/mman.h>
#include <signal.h>

#include "../../common/test.h"

int pipe_1[2], pipe_2[2];
int sock[2];
int epoll_fd, event_fd, timer_fd, signal_fd, inotify_fd, pid_fd, mem_fd;
pid_t child;

FN_SETUP(create)
{
	CHECK(pipe(pipe_1));
	CHECK(pipe(pipe_2));
	CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, sock));

	epoll_fd = CHECK(epoll_create1(EPOLL_CLOEXEC));
	event_fd = CHECK(eventfd(0, EFD_CLOEXEC));
	timer_fd = CHECK(timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC));
	inotify_fd = CHECK(inotify_init1(0));
	mem_fd = CHECK(memfd_create("test_memfd", MFD_CLOEXEC));

	sigset_t mask;
	CHECK(sigemptyset(&mask));
	CHECK(sigaddset(&mask, SIGUSR1));
	signal_fd = CHECK(signalfd(-1, &mask, SFD_CLOEXEC));

	child = CHECK(fork());
	if (child == 0) {
		pause();
		exit(-1);
	}
	pid_fd = CHECK(syscall(SYS_pidfd_open, child, 0));
}
END_SETUP()

static void __attribute__((unused)) fd_path(int fd, char *buf, size_t buflen)
{
	CHECK_WITH(snprintf(buf, buflen, "/proc/self/fd/%d", fd),
		   _ret > 0 && _ret < buflen);
}

static void __attribute__((unused))
fdinfo_path(int fd, char *buf, size_t buflen)
{
	CHECK_WITH(snprintf(buf, buflen, "/proc/self/fdinfo/%d", fd),
		   _ret > 0 && _ret < buflen);
}
