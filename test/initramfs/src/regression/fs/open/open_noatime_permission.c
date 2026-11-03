// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TARGET "/tmp/open_noatime_permission_test"

FN_TEST(noatime_requires_owner_or_capability)
{
	int fd, status;
	pid_t pid;
	struct stat st;

	fd = CHECK(open(TARGET, O_CREAT | O_EXCL | O_RDWR, 0600));
	CHECK_WITH(write(fd, "x", 1), _ret == 1);
	CHECK(fchmod(fd, 0666));
	CHECK(fchown(fd, 65533, 65533));
	CHECK(close(fd));

	/* A non-owner with CAP_FOWNER may use O_NOATIME. */
	fd = TEST_RES(open(TARGET, O_RDONLY | O_NOATIME), _ret >= 0);
	if (fd >= 0)
		CHECK(close(fd));

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(setresgid(65533, 65533, 65533));
		CHECK(setresuid(65533, 65533, 65533));
		fd = CHECK(open(TARGET, O_RDONLY | O_NOATIME));
		CHECK(close(fd));
		_exit(0);
	}
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(setresgid(65534, 65534, 65534));
		CHECK(setresuid(65534, 65534, 65534));
		fd = CHECK(open(TARGET, O_RDONLY));
		CHECK(close(fd));
		CHECK_WITH(open(TARGET, O_RDONLY | O_NOATIME),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(open(TARGET, O_WRONLY | O_TRUNC | O_NOATIME),
			   _ret == -1 && errno == EPERM);
		_exit(0);
	}
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	CHECK(stat(TARGET, &st));
	TEST_RES(st.st_size, _ret == 1);
	CHECK(unlink(TARGET));
}
END_TEST()
