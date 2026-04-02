/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"

#define POLICY_FILE "/test/security/aster/open_target"
#define EXEC_PROBE "/test/security/aster/exec_probe"

static void clear_policy_xattr(const char *path, const char *name)
{
	if (removexattr(path, name) == 0 || errno == ENODATA) {
		return;
	}

	perror("removexattr");
	exit(EXIT_FAILURE);
}

static int set_policy_xattr(const char *path, const char *name, const char *value)
{
	return setxattr(path, name, value, strlen(value), 0);
}

static int expect_exec_denied(void)
{
	pid_t pid = fork();
	if (pid < 0) {
		return -1;
	}

	if (pid == 0) {
		execl(EXEC_PROBE, EXEC_PROBE, NULL);
		_exit(errno == EACCES ? 100 : 101);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		return -1;
	}

	if (WIFEXITED(status) && WEXITSTATUS(status) == 100) {
		errno = EACCES;
		return -1;
	}

	errno = EIO;
	return -1;
}

static int expect_exec_allowed(void)
{
	pid_t pid = fork();
	if (pid < 0) {
		return -1;
	}

	if (pid == 0) {
		execl(EXEC_PROBE, EXEC_PROBE, NULL);
		_exit(111);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		return -1;
	}

	if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
		return 0;
	}

	errno = EIO;
	return -1;
}

FN_SETUP(prepare_policy_target)
{
	clear_policy_xattr(POLICY_FILE, "security.aster.open");
	clear_policy_xattr(POLICY_FILE, "security.aster.read");
	clear_policy_xattr(POLICY_FILE, "security.aster.write");
	clear_policy_xattr(EXEC_PROBE, "security.aster.exec");

	int fd = CHECK(open(POLICY_FILE, O_CREAT | O_RDWR | O_TRUNC, 0700));
	const char payload[] = "aster-lsm";
	CHECK(write(fd, payload, sizeof(payload) - 1));
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(open_xattr_blocks_o_path_open)
{
	TEST_SUCC(set_policy_xattr(POLICY_FILE, "security.aster.open", "1"));
	TEST_ERRNO(open(POLICY_FILE, O_PATH), EACCES);
	TEST_SUCC(removexattr(POLICY_FILE, "security.aster.open"));
	int fd = TEST_RES(open(POLICY_FILE, O_PATH), _ret >= 0);
	if (fd >= 0) {
		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(read_xattr_blocks_access_and_open)
{
	TEST_SUCC(set_policy_xattr(POLICY_FILE, "security.aster.read", "1"));
	TEST_ERRNO(access(POLICY_FILE, R_OK), EACCES);
	TEST_ERRNO(open(POLICY_FILE, O_RDONLY), EACCES);
	TEST_SUCC(removexattr(POLICY_FILE, "security.aster.read"));
	TEST_SUCC(access(POLICY_FILE, R_OK));
	int fd = TEST_RES(open(POLICY_FILE, O_RDONLY), _ret >= 0);
	if (fd >= 0) {
		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(write_xattr_blocks_access_and_open)
{
	TEST_SUCC(set_policy_xattr(POLICY_FILE, "security.aster.write", "1"));
	TEST_ERRNO(access(POLICY_FILE, W_OK), EACCES);
	TEST_ERRNO(open(POLICY_FILE, O_WRONLY), EACCES);
	TEST_SUCC(removexattr(POLICY_FILE, "security.aster.write"));
	TEST_SUCC(access(POLICY_FILE, W_OK));
	int fd = TEST_RES(open(POLICY_FILE, O_WRONLY), _ret >= 0);
	if (fd >= 0) {
		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(exec_xattr_blocks_execve)
{
	TEST_SUCC(set_policy_xattr(EXEC_PROBE, "security.aster.exec", "1"));
	TEST_ERRNO(expect_exec_denied(), EACCES);
	TEST_SUCC(removexattr(EXEC_PROBE, "security.aster.exec"));
	TEST_SUCC(expect_exec_allowed());
}
END_TEST()
