// SPDX-License-Identifier: MPL-2.0

#include "pseudo_file_create.h"

static int readlink_check(int fd, const char *expect_prefix, int check_ino)
{
	char path[64];
	char buf[256];
	struct stat st;

	fd_path(fd, path, sizeof(path));
	ssize_t len = readlink(path, buf, sizeof(buf) - 1);
	if (len < 0)
		return -1;
	buf[len] = '\0';

	size_t prefix_len = strlen(expect_prefix);
	if (strncmp(buf, expect_prefix, prefix_len) != 0)
		return -1;

	if (check_ino) {
		const char *ino_p = buf + prefix_len;
		char *end = NULL;

		unsigned long ino = strtoul(ino_p, &end, 10);
		if (end == ino_p || *end != ']' || *(end + 1) != '\0')
			return -1;
		if (fstat(fd, &st) < 0 || st.st_ino != ino)
			return -1;
	}

	return 0;
}

FN_TEST(pseudo_dentry)
{
	TEST_RES(readlink_check(pipe_1[0], "pipe:[", 1), _ret == 0);
	TEST_RES(readlink_check(pipe_1[1], "pipe:[", 1), _ret == 0);

	TEST_RES(readlink_check(sock[0], "socket:[", 1), _ret == 0);
	TEST_RES(readlink_check(sock[1], "socket:[", 1), _ret == 0);

	TEST_RES(readlink_check(epoll_fd, "anon_inode:[eventpoll]", 0),
		 _ret == 0);
	TEST_RES(readlink_check(event_fd, "anon_inode:[eventfd]", 0),
		 _ret == 0);
	TEST_RES(readlink_check(timer_fd, "anon_inode:[timerfd]", 0),
		 _ret == 0);
	TEST_RES(readlink_check(signal_fd, "anon_inode:[signalfd]", 0),
		 _ret == 0);
	TEST_RES(readlink_check(inotify_fd, "anon_inode:inotify", 0),
		 _ret == 0);
	TEST_RES(readlink_check(pid_fd, "anon_inode:[pidfd]", 0), _ret == 0);

	TEST_RES(readlink_check(mem_fd, "/memfd:test_memfd", 0), _ret == 0);
}
END_TEST()

#include "pseudo_file_cleanup.h"
