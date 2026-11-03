// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TARGET "/tmp/open_trunc_permission_test"

static int check_open(int flags, int succeeds)
{
	const char data[] = "original data";
	char buffer[sizeof(data)] = { 0 };
	struct stat st;
	int fd, status, passed;
	pid_t pid;

	fd = CHECK(open(TARGET, O_CREAT | O_EXCL | O_WRONLY, 0600));
	CHECK_WITH(write(fd, data, sizeof(data)), _ret == sizeof(data));
	CHECK(fchmod(fd, 0222));
	CHECK(close(fd));

	pid = CHECK(fork());
	if (pid == 0) {
		CHECK(setresgid(65534, 65534, 65534));
		CHECK(setresuid(65534, 65534, 65534));
		if (succeeds) {
			fd = open(TARGET, flags | O_TRUNC);
			if (fd < 0)
				_exit(1);
			CHECK(close(fd));
		} else {
			fd = open(TARGET, flags | O_TRUNC);
			if (fd >= 0) {
				CHECK(close(fd));
				_exit(1);
			}
			if (errno != EACCES)
				_exit(1);
		}
		_exit(0);
	}
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid);
	passed = WIFEXITED(status) && WEXITSTATUS(status) == 0;
	CHECK(stat(TARGET, &st));
	passed = passed && st.st_size == (succeeds ? 0 : sizeof(data));
	if (!succeeds) {
		fd = CHECK(open(TARGET, O_RDONLY));
		if (passed)
			passed = CHECK(read(fd, buffer, sizeof(buffer))) ==
				 sizeof(buffer);
		passed = passed && memcmp(buffer, data, sizeof(data)) == 0;
		CHECK(close(fd));
	}
	CHECK(unlink(TARGET));
	return passed;
}

FN_TEST(truncation_requires_all_requested_permissions)
{
	TEST_RES(check_open(O_RDONLY, 0), _ret == 1);
	TEST_RES(check_open(O_RDWR, 0), _ret == 1);
	TEST_RES(check_open(O_WRONLY, 1), _ret == 1);
}
END_TEST()
