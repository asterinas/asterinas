// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/capability.h"

#define SMACK_LABEL_MAX_LEN 255
#define SMACK_ATTR_CURRENT_MAX_LEN (SMACK_LABEL_MAX_LEN + 1)
#define SMACK_XATTR_ACCESS "security.SMACK64"
#define SMACK_XATTR_EXEC "security.SMACK64EXEC"
#define SMACK_XATTR_TRANSMUTE "security.SMACK64TRANSMUTE"

static void build_attr_current_path(char *buf, size_t size)
{
	CHECK_WITH(snprintf(buf, size, "/proc/%ld/task/%ld/attr/current",
			    (long)getpid(), (long)syscall(SYS_gettid)),
		   _ret > 0 && (size_t)_ret < size);
}

static int read_current_label(char *buf, size_t size)
{
	char path[128];
	int saved_errno;
	int fd;
	ssize_t len;
	char *newline;

	build_attr_current_path(path, sizeof(path));
	fd = open(path, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	len = read(fd, buf, size - 1);
	saved_errno = errno;
	close(fd);
	errno = saved_errno;
	if (len < 0) {
		return -1;
	}

	buf[len] = '\0';
	newline = strchr(buf, '\n');
	if (newline != NULL) {
		*newline = '\0';
	}

	return 0;
}

static int write_current_label(const char *label)
{
	char path[128];
	size_t len = strlen(label);
	int fd;
	ssize_t written;
	int saved_errno;

	build_attr_current_path(path, sizeof(path));
	fd = open(path, O_WRONLY);
	if (fd < 0) {
		return -1;
	}

	written = write(fd, label, len);
	saved_errno = errno;
	close(fd);
	errno = saved_errno;
	if (written != (ssize_t)len) {
		errno = written < 0 ? saved_errno : EIO;
		return -1;
	}

	return 0;
}

static bool smack_is_disabled(void)
{
	char label[SMACK_ATTR_CURRENT_MAX_LEN] = {};

	if (read_current_label(label, sizeof(label)) == 0) {
		return false;
	}

	return errno == ENOENT;
}

static int write_all(int fd, const void *buf, size_t len)
{
	const char *cursor = buf;

	while (len > 0) {
		ssize_t written = write(fd, cursor, len);
		if (written < 0) {
			return -1;
		}
		if (written == 0) {
			errno = EIO;
			return -1;
		}
		cursor += written;
		len -= written;
	}

	return 0;
}

static int copy_self_to(const char *path)
{
	char buf[4096];
	int src = open("/proc/self/exe", O_RDONLY);
	int dst;

	if (src < 0) {
		return -1;
	}

	dst = open(path, O_CREAT | O_WRONLY | O_TRUNC, 0755);
	if (dst < 0) {
		close(src);
		return -1;
	}

	for (;;) {
		ssize_t len = read(src, buf, sizeof(buf));
		if (len < 0) {
			close(src);
			close(dst);
			return -1;
		}
		if (len == 0) {
			break;
		}
		if (write_all(dst, buf, len) < 0) {
			close(src);
			close(dst);
			return -1;
		}
	}

	if (close(src) < 0) {
		close(dst);
		return -1;
	}
	if (close(dst) < 0) {
		return -1;
	}

	return chmod(path, 0755);
}

static void run_exec_helper_if_requested(void) __attribute__((constructor(101)));

static void run_exec_helper_if_requested(void)
{
	const char *fd_string = getenv("SMACK_HELPER_FD");
	char label[SMACK_ATTR_CURRENT_MAX_LEN] = {};
	int fd;

	if (fd_string == NULL) {
		return;
	}

	fd = atoi(fd_string);
	if (read_current_label(label, sizeof(label)) < 0) {
		_exit(EXIT_FAILURE);
	}
	if (write_all(fd, label, strlen(label)) < 0) {
		_exit(EXIT_FAILURE);
	}

	_exit(EXIT_SUCCESS);
}

FN_TEST(attr_current_roundtrip)
{
	pid_t pid;
	int status;

	SKIP_TEST_IF(smack_is_disabled());

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char label[SMACK_ATTR_CURRENT_MAX_LEN] = {};

		CHECK(read_current_label(label, sizeof(label)));
		CHECK_WITH(strcmp(label, "_"), _ret == 0);
		CHECK(write_current_label("smack_foundation\n"));
		CHECK(read_current_label(label, sizeof(label)));
		CHECK_WITH(strcmp(label, "smack_foundation"), _ret == 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(xattr_validation_and_capability)
{
	pid_t pid;
	int status;

	SKIP_TEST_IF(smack_is_disabled());

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char file_template[] = "/tmp/smack_xattrXXXXXX";
		int file = CHECK(mkstemp(file_template));

		CHECK(close(file));
		CHECK_WITH(setxattr(file_template, SMACK_XATTR_ACCESS, "bad/name",
				    strlen("bad/name"), 0),
			   _ret == -1 && errno == EINVAL);
		CHECK(setxattr(file_template, SMACK_XATTR_ACCESS, "smack_file",
			       strlen("smack_file"), 0));
		CHECK_WITH(setxattr(file_template, SMACK_XATTR_TRANSMUTE, "FALSE",
				    strlen("FALSE"), 0),
			   _ret == -1 && errno == EINVAL);
		CHECK(setxattr(file_template, SMACK_XATTR_TRANSMUTE, "TRUE",
			       strlen("TRUE"), 0));
		CHECK(removexattr(file_template, SMACK_XATTR_TRANSMUTE));

		drop_capability(CAP_MAC_ADMIN);

		errno = 0;
		CHECK_WITH(setxattr(file_template, SMACK_XATTR_ACCESS, "other",
				    strlen("other"), 0),
			   _ret == -1 && errno == EPERM);
		errno = 0;
		CHECK_WITH(removexattr(file_template, SMACK_XATTR_ACCESS),
			   _ret == -1 && errno == EPERM);

		CHECK(unlink(file_template));
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(exec_label_transition)
{
	char helper_path[64];
	int pipefd[2];
	pid_t pid;
	int status;
	char label[SMACK_ATTR_CURRENT_MAX_LEN] = {};
	ssize_t len;

	SKIP_TEST_IF(smack_is_disabled());

	CHECK_WITH(snprintf(helper_path, sizeof(helper_path),
			    "/tmp/smack_exec_%ld", (long)getpid()),
		   _ret > 0 && (size_t)_ret < sizeof(helper_path));
	TEST_SUCC(copy_self_to(helper_path));
	TEST_SUCC(setxattr(helper_path, SMACK_XATTR_EXEC, "smack_exec",
			   strlen("smack_exec"), 0));
	TEST_SUCC(pipe(pipefd));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char fd_string[16];

		CHECK(close(pipefd[0]));
		CHECK_WITH(snprintf(fd_string, sizeof(fd_string), "%d", pipefd[1]),
			   _ret > 0 && (size_t)_ret < sizeof(fd_string));
		CHECK(setenv("SMACK_HELPER_FD", fd_string, 1));
		execl(helper_path, helper_path, NULL);
		_exit(EXIT_FAILURE);
	}

	TEST_SUCC(close(pipefd[1]));
	len = TEST_RES(read(pipefd[0], label, sizeof(label) - 1), _ret > 0);
	label[len] = '\0';
	TEST_SUCC(close(pipefd[0]));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(strcmp(label, "smack_exec"), _ret == 0);

	TEST_SUCC(removexattr(helper_path, SMACK_XATTR_EXEC));
	TEST_SUCC(unlink(helper_path));
}
END_TEST()
