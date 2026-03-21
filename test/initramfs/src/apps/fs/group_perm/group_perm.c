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

#define TEST_DIR "/ext2/group_perm_test"
#define TARGET_FILE TEST_DIR "/group_readable"
#define TEST_UID 1000
#define TEST_GID 1000
#define ACCESS_GID 2000

static int open_as_unprivileged_with_groups(const char *path, int flags,
					    mode_t mode, const gid_t *groups,
					    size_t group_count)
{
	pid_t child = CHECK(fork());

	if (child == 0) {
		if (setgroups(group_count, groups) < 0) {
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

	CHECK(mkdir(TEST_DIR, 0750));
	fd = CHECK(open(TARGET_FILE, O_CREAT | O_RDWR | O_TRUNC, 0640));
	CHECK(close(fd));

	CHECK(chown(TEST_DIR, 0, ACCESS_GID));
	CHECK(chmod(TEST_DIR, 0750));
	CHECK(chown(TARGET_FILE, 0, ACCESS_GID));
	CHECK(chmod(TARGET_FILE, 0640));
}
END_SETUP()

FN_TEST(open_without_supplementary_group_returns_eacces)
{
	TEST_ERRNO(open_as_unprivileged_with_groups(TARGET_FILE, O_RDONLY, 0,
						    NULL, 0),
		   EACCES);
}
END_TEST()

FN_TEST(open_with_supplementary_group_succeeds)
{
	gid_t groups[] = { ACCESS_GID };

	TEST_SUCC(open_as_unprivileged_with_groups(
		TARGET_FILE, O_RDONLY, 0, groups,
		sizeof(groups) / sizeof(groups[0])));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(TARGET_FILE));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
