// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <fcntl.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/eventfd.h>
#include <sys/timerfd.h>
#include <sys/signalfd.h>
#include <sys/epoll.h>
#include <signal.h>

#include "../test.h"

static void fd_path(int fd, char *buf, size_t buflen)
{
	CHECK_WITH(snprintf(buf, buflen, "/proc/self/fd/%d", fd),
		   _ret > 0 && _ret < buflen);
}

static int get_mode(int fd)
{
	char path[64];
	struct stat st;

	fd_path(fd, path, sizeof(path));
	if (stat(path, &st) < 0)
		return -1;

	return st.st_mode & 0777;
}

static int set_mode(int fd, int mode)
{
	char path[64];

	fd_path(fd, path, sizeof(path));
	return chmod(path, mode & 0777);
}

FN_TEST(pipe_ends_share_inode)
{
	int pipe1[2], pipe2[2];
	TEST_SUCC(pipe(pipe1));
	TEST_SUCC(pipe(pipe2));

	TEST_RES(get_mode(pipe1[0]), _ret == 0600);
	TEST_RES(get_mode(pipe1[1]), _ret == 0600);
	TEST_RES(get_mode(pipe2[0]), _ret == 0600);
	TEST_RES(get_mode(pipe2[1]), _ret == 0600);

	TEST_SUCC(set_mode(pipe1[0], 0000));

	TEST_RES(get_mode(pipe1[0]), _ret == 0000);
	TEST_RES(get_mode(pipe1[1]), _ret == 0000);
	TEST_RES(get_mode(pipe2[0]), _ret == 0600);
	TEST_RES(get_mode(pipe2[1]), _ret == 0600);
}
END_TEST()

FN_TEST(sockets_do_not_share_inode)
{
	int sock[2];
	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, sock));

	TEST_RES(get_mode(sock[0]), _ret == 0777);
	TEST_RES(get_mode(sock[1]), _ret == 0777);

	TEST_SUCC(set_mode(sock[0], 0000));

	TEST_RES(get_mode(sock[0]), _ret == 0000);
	TEST_RES(get_mode(sock[1]), _ret == 0777);
}
END_TEST()

FN_TEST(anon_inodefs_share_inode)
{
	int fd;

	// eventfd
	fd = TEST_SUCC(eventfd(0, EFD_CLOEXEC));
	TEST_RES(get_mode(fd), _ret == 0600);
	TEST_SUCC(set_mode(fd, 0000));
	TEST_RES(get_mode(fd), _ret == 0000);
	TEST_SUCC(close(fd));

	// timerfd
	fd = TEST_SUCC(timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC));
	TEST_RES(get_mode(fd), _ret == 0000);
	TEST_SUCC(set_mode(fd, 0111));
	TEST_RES(get_mode(fd), _ret == 0111);
	TEST_SUCC(close(fd));

	// signalfd
	sigset_t mask;
	TEST_SUCC(sigemptyset(&mask));
	TEST_SUCC(sigaddset(&mask, SIGUSR1));
	fd = TEST_SUCC(signalfd(-1, &mask, SFD_CLOEXEC));
	TEST_RES(get_mode(fd), _ret == 0111);
	TEST_SUCC(set_mode(fd, 0222));
	TEST_RES(get_mode(fd), _ret == 0222);
	TEST_SUCC(close(fd));

	// epollfd
	fd = TEST_SUCC(epoll_create1(EPOLL_CLOEXEC));
	TEST_RES(get_mode(fd), _ret == 0222);
	TEST_SUCC(set_mode(fd, 0600));
	TEST_RES(get_mode(fd), _ret == 0600);
	TEST_SUCC(close(fd));
}
END_TEST()
