// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ext2/open_dir_test"
#define EXISTING_DIR TEST_DIR "/objd"
#define TARGET_FILE TEST_DIR "/regf"
#define TARGET_LINK TEST_DIR "/lnk"
#define LOOP_LINK TEST_DIR "/loop"
#define LOCKED_DIR TEST_DIR "/locked"
#define LOCKED_CHILD LOCKED_DIR "/new"
#define TEST_UID 1000
#define TEST_GID 1000

static int open_as_unprivileged_user(const char *path, int flags, mode_t mode)
{
	pid_t child = CHECK(fork());

	if (child == 0) {
		if (setgroups(0, NULL) < 0) {
			_exit(errno);
		}
		if (setresgid(TEST_GID, TEST_GID, TEST_GID) < 0) {
			_exit(errno);
		}
		if (setresuid(TEST_UID, TEST_UID, TEST_UID) < 0) {
			_exit(errno);
		}

		int fd = open(path, flags, mode);
		if (fd < 0) {
			_exit(errno);
		}
		if (close(fd) < 0) {
			_exit(errno);
		}

		_exit(0);
	}

	int status = 0;
	CHECK_WITH(waitpid(child, &status, 0), WIFEXITED(status));

	int child_status = WEXITSTATUS(status);
	if (child_status == 0) {
		errno = 0;
		return 0;
	}

	errno = child_status;
	return -1;
}

FN_SETUP(prepare)
{
	int fd = -1;

	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(mkdir(EXISTING_DIR, 0755));

	fd = CHECK(open(TARGET_FILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
	CHECK(close(fd));
	CHECK(symlink("regf", TARGET_LINK));
	CHECK(symlink("loop", LOOP_LINK));
	CHECK(mkdir(LOCKED_DIR, 0700));
}
END_SETUP()

FN_TEST(open_creat_directory_on_existing_dir_returns_einval)
{
	TEST_ERRNO(open(EXISTING_DIR, O_CREAT | O_DIRECTORY | O_RDONLY, 0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_on_existing_dir_returns_eisdir)
{
	TEST_ERRNO(open(EXISTING_DIR, O_CREAT | O_WRONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_excl_on_existing_dir_returns_eexist)
{
	TEST_ERRNO(open(EXISTING_DIR, O_CREAT | O_EXCL | O_WRONLY, 0644),
		   EEXIST);
}
END_TEST()

FN_TEST(open_creat_excl_on_existing_dir_with_trailing_slash_returns_eisdir)
{
	TEST_ERRNO(open(EXISTING_DIR "/", O_CREAT | O_EXCL | O_WRONLY, 0644),
		   EISDIR);
}
END_TEST()

FN_TEST(open_write_only_on_dir_returns_eisdir)
{
	TEST_ERRNO(open(EXISTING_DIR, O_WRONLY), EISDIR);
}
END_TEST()

FN_TEST(open_rdwr_on_dir_returns_eisdir)
{
	TEST_ERRNO(open(EXISTING_DIR, O_RDWR), EISDIR);
}
END_TEST()

FN_TEST(open_directory_nofollow_symlink_returns_enotdir)
{
	TEST_ERRNO(open(TARGET_LINK, O_RDONLY | O_DIRECTORY | O_NOFOLLOW),
		   ENOTDIR);
}
END_TEST()

FN_TEST(open_creat_on_symlink_with_trailing_slash_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_LINK "/", O_CREAT | O_RDONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_on_symlink_loop_with_trailing_slash_returns_eisdir)
{
	TEST_ERRNO(open(LOOP_LINK "/", O_CREAT | O_RDONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_directory_on_inaccessible_path_returns_einval)
{
	TEST_ERRNO(open_as_unprivileged_user(LOCKED_CHILD,
					     O_CREAT | O_DIRECTORY | O_RDONLY,
					     0644),
		   EINVAL);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(LOOP_LINK));
	CHECK(unlink(TARGET_LINK));
	CHECK(unlink(TARGET_FILE));
	CHECK(rmdir(LOCKED_DIR));
	CHECK(rmdir(EXISTING_DIR));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
