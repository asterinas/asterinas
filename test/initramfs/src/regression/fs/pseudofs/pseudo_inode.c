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

static const uid_t creator_uid = 65534;
static const gid_t creator_gid = 65533;

static void check_pseudo_inode_owner_and_mode(int fd)
{
	struct stat st;

	CHECK_WITH(fstat(fd, &st), _ret == 0 && st.st_uid == creator_uid &&
					   st.st_gid == creator_gid);
	CHECK(fchmod(fd, 02640));
	CHECK_WITH(fstat(fd, &st), _ret == 0 && (st.st_mode & 07777) == 02640);
}

// Pipefs and sockfs inodes use the creator's filesystem IDs, as Linux does.
FN_TEST(pseudo_inodes_use_creator_filesystem_ids)
{
	pid_t owner_child = TEST_SUCC(fork());
	if (owner_child == 0) {
		int pipe_fds[2], socket_fds[2];

		CHECK(setgid(creator_gid));
		CHECK(setuid(creator_uid));

		CHECK(pipe(pipe_fds));
		CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, socket_fds));
		check_pseudo_inode_owner_and_mode(pipe_fds[0]);
		check_pseudo_inode_owner_and_mode(pipe_fds[1]);
		check_pseudo_inode_owner_and_mode(socket_fds[0]);
		check_pseudo_inode_owner_and_mode(socket_fds[1]);
		_exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(waitpid(owner_child, &status, 0),
		 _ret == owner_child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
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

FN_TEST(nsfs_inode)
{
	TEST_RES(get_mode(ns_uts_fd), _ret == 0444);
	TEST_ERRNO(set_mode(ns_uts_fd, 0600), EPERM);
}
END_TEST()

#include "pseudo_file_cleanup.h"
