// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define STICKY_DIR "/ext2/sticky_test"
#define STICKY_FILE STICKY_DIR "/rootfile"
#define STICKY_EMPTY_DIR STICKY_DIR "/emptydir"
#define STICKY_NONEMPTY_DIR STICKY_DIR "/nonemptydir"
#define STICKY_NONEMPTY_CHILD STICKY_NONEMPTY_DIR "/inner"

#define NON_OWNER_UID 1000

static void remove_paths_if_exist(void)
{
	CHECK_WITH(unlink(STICKY_NONEMPTY_CHILD), _ret >= 0 || errno == ENOENT);
	CHECK_WITH(rmdir(STICKY_NONEMPTY_DIR), _ret >= 0 || errno == ENOENT);
	CHECK_WITH(rmdir(STICKY_EMPTY_DIR), _ret >= 0 || errno == ENOENT);
	CHECK_WITH(unlink(STICKY_FILE), _ret >= 0 || errno == ENOENT);
	CHECK_WITH(rmdir(STICKY_DIR), _ret >= 0 || errno == ENOENT);
}

static void prepare_sticky_dir(void)
{
	remove_paths_if_exist();

	CHECK(mkdir(STICKY_DIR, 0777));
	CHECK(chmod(STICKY_DIR, 01777));

	int fd = CHECK(creat(STICKY_FILE, 0644));
	CHECK(close(fd));
	CHECK(mkdir(STICKY_EMPTY_DIR, 0755));
	CHECK(mkdir(STICKY_NONEMPTY_DIR, 0755));
	fd = CHECK(creat(STICKY_NONEMPTY_CHILD, 0644));
	CHECK(close(fd));
}

static void become_non_owner(void)
{
	CHECK(setresgid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
	CHECK(setresuid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
}

FN_TEST(unlink_in_sticky_dir_requires_ownership)
{
	prepare_sticky_dir();

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK_WITH(unlink(STICKY_FILE), _ret == -1 && errno == EPERM);
		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	struct stat st;
	TEST_SUCC(stat(STICKY_FILE, &st));

	remove_paths_if_exist();
}
END_TEST()

FN_TEST(rmdir_in_sticky_dir_requires_ownership)
{
	prepare_sticky_dir();

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK_WITH(rmdir(STICKY_EMPTY_DIR),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(rmdir(STICKY_NONEMPTY_DIR),
			   _ret == -1 && errno == EPERM);
		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	struct stat st;
	TEST_SUCC(stat(STICKY_EMPTY_DIR, &st));

	remove_paths_if_exist();
}
END_TEST()

FN_TEST(sticky_dir_allows_removing_own_entries)
{
	prepare_sticky_dir();

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		int fd = CHECK(
			open(STICKY_DIR "/ownfile", O_CREAT | O_WRONLY, 0644));
		CHECK(close(fd));
		CHECK(unlink(STICKY_DIR "/ownfile"));
		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	remove_paths_if_exist();
}
END_TEST()

FN_TEST(sticky_dir_does_not_affect_plain_directory)
{
	remove_paths_if_exist();

	TEST_SUCC(mkdir(STICKY_DIR, 0777));
	TEST_SUCC(chmod(STICKY_DIR, 0777));
	int fd = TEST_SUCC(creat(STICKY_FILE, 0644));
	TEST_SUCC(close(fd));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK_WITH(unlink(STICKY_FILE), _ret == 0);
		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	remove_paths_if_exist();
}
END_TEST()
