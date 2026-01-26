// SPDX-License-Identifier: MPL-2.0

#include "pseudo_file_create.h"

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
	TEST_RES(get_mode(pipe_1[0]), _ret == 0600);
	TEST_RES(get_mode(pipe_1[1]), _ret == 0600);
	TEST_RES(get_mode(pipe_2[0]), _ret == 0600);
	TEST_RES(get_mode(pipe_2[1]), _ret == 0600);

	TEST_SUCC(set_mode(pipe_1[0], 0000));

	TEST_RES(get_mode(pipe_1[0]), _ret == 0000);
	TEST_RES(get_mode(pipe_1[1]), _ret == 0000);
	TEST_RES(get_mode(pipe_2[0]), _ret == 0600);
	TEST_RES(get_mode(pipe_2[1]), _ret == 0600);
}
END_TEST()

FN_TEST(sockets_do_not_share_inode)
{
	TEST_RES(get_mode(sock[0]), _ret == 0777);
	TEST_RES(get_mode(sock[1]), _ret == 0777);

	TEST_SUCC(set_mode(sock[0], 0000));

	TEST_RES(get_mode(sock[0]), _ret == 0000);
	TEST_RES(get_mode(sock[1]), _ret == 0777);
}
END_TEST()

FN_TEST(anon_inodefs_share_inode)
{
	struct fd_mode {
		int fd;
		mode_t mode;
	};

	struct fd_mode fds[] = {
		{ epoll_fd, 0600 },  { event_fd, 0600 },   { timer_fd, 0600 },
		{ signal_fd, 0600 }, { inotify_fd, 0600 }, { pid_fd, 0700 },
	};

	for (size_t i = 0; i < sizeof(fds) / sizeof(fds[0]); i++) {
		TEST_RES(get_mode(fds[i].fd), _ret == fds[i].mode);
		TEST_ERRNO(set_mode(fds[i].fd, 0600), EOPNOTSUPP);
	}
}
END_TEST()

#include "pseudo_file_cleanup.h"
