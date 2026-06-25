// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../../common/test.h"

#define FDINFO_O_LARGEFILE 0100000

static unsigned int read_fdinfo_flags(int fd)
{
	char path[64];
	char buf[512] = { 0 };
	unsigned int flags = 0;

	CHECK_WITH(snprintf(path, sizeof(path), "/proc/self/fdinfo/%d", fd),
		   _ret > 0 && (size_t)_ret < sizeof(path));

	int info_fd = CHECK(open(path, O_RDONLY));
	CHECK(read(info_fd, buf, sizeof(buf) - 1));
	CHECK(close(info_fd));

	char *flags_line = strstr(buf, "flags:\t0");
	CHECK(flags_line == NULL ? -1 : 0);
	CHECK_WITH(sscanf(flags_line, "flags:\t0%o", &flags), _ret == 1);

	return flags;
}

FN_TEST(regular_file_fdinfo_reports_largefile)
{
	const char *path = "/tmp/fdinfo_largefile_regular";
	int fd = TEST_SUCC(
		open(path, O_CREAT | O_RDWR | O_TRUNC | O_CLOEXEC | O_NONBLOCK,
		     0600));

	unsigned int flags = read_fdinfo_flags(fd);
	int fcntl_flags = TEST_SUCC(fcntl(fd, F_GETFL));

	TEST_RES(flags & FDINFO_O_LARGEFILE, _ret != 0);
	TEST_RES(flags & O_CLOEXEC, _ret != 0);
	TEST_RES(flags & O_NONBLOCK, _ret != 0);
	TEST_RES((flags & O_ACCMODE) == O_RDWR, _ret == 1);
	TEST_RES(fcntl_flags & FDINFO_O_LARGEFILE, _ret != 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(named_pipe_fdinfo_reports_largefile)
{
	const char *path = "/tmp/fdinfo_largefile_fifo";

	unlink(path);
	TEST_SUCC(mkfifo(path, 0600));
	int fd = TEST_SUCC(open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC));

	unsigned int flags = read_fdinfo_flags(fd);
	int fcntl_flags = TEST_SUCC(fcntl(fd, F_GETFL));

	TEST_RES(flags & FDINFO_O_LARGEFILE, _ret != 0);
	TEST_RES(flags & O_CLOEXEC, _ret != 0);
	TEST_RES(flags & O_NONBLOCK, _ret != 0);
	TEST_RES((flags & O_ACCMODE) == O_RDONLY, _ret == 1);
	TEST_RES(fcntl_flags & FDINFO_O_LARGEFILE, _ret != 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(opath_fgetfl_does_not_report_largefile)
{
	const char *path = "/tmp/fdinfo_largefile_opath";
	int create_fd =
		TEST_SUCC(open(path, O_CREAT | O_RDONLY | O_TRUNC, 0600));
	TEST_SUCC(close(create_fd));

	int fd = TEST_SUCC(open(path, O_PATH | O_CLOEXEC));
	unsigned int flags = read_fdinfo_flags(fd);
	int fcntl_flags = TEST_SUCC(fcntl(fd, F_GETFL));

	TEST_RES(flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(fcntl_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(fcntl_flags & O_PATH, _ret != 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(anonymous_pipe_fgetfl_does_not_report_largefile)
{
	int pipefd[2];
	TEST_SUCC(pipe(pipefd));

	int read_flags = TEST_SUCC(fcntl(pipefd[0], F_GETFL));
	int write_flags = TEST_SUCC(fcntl(pipefd[1], F_GETFL));
	unsigned int read_info_flags = read_fdinfo_flags(pipefd[0]);
	unsigned int write_info_flags = read_fdinfo_flags(pipefd[1]);

	TEST_RES(read_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(write_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(read_info_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(write_info_flags & FDINFO_O_LARGEFILE, _ret == 0);

	TEST_SUCC(close(pipefd[0]));
	TEST_SUCC(close(pipefd[1]));
}
END_TEST()

FN_TEST(socket_fdinfo_does_not_report_largefile)
{
	int socketfd[2];
	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, socketfd));

	int first_flags = TEST_SUCC(fcntl(socketfd[0], F_GETFL));
	int second_flags = TEST_SUCC(fcntl(socketfd[1], F_GETFL));
	unsigned int first_fdinfo_flags = read_fdinfo_flags(socketfd[0]);
	unsigned int second_fdinfo_flags = read_fdinfo_flags(socketfd[1]);

	TEST_RES(first_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(second_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(first_fdinfo_flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(second_fdinfo_flags & FDINFO_O_LARGEFILE, _ret == 0);

	TEST_SUCC(close(socketfd[0]));
	TEST_SUCC(close(socketfd[1]));
}
END_TEST()

FN_TEST(eventfd_fdinfo_does_not_report_largefile)
{
	int fd = TEST_SUCC(eventfd(0, EFD_CLOEXEC));
	unsigned int flags = read_fdinfo_flags(fd);
	int fcntl_flags = TEST_SUCC(fcntl(fd, F_GETFL));

	TEST_RES(flags & FDINFO_O_LARGEFILE, _ret == 0);
	TEST_RES(flags & O_CLOEXEC, _ret != 0);
	TEST_RES(fcntl_flags & FDINFO_O_LARGEFILE, _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()
