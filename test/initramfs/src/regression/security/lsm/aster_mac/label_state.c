/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../../common/test.h"

#define CURRENT_LABEL_FILE "/proc/self/attr/current"
#define LABEL_PROBE_SOURCE "/test/security/lsm/aster_mac/label_probe"
#define LABEL_PROBE "/tmp/aster_mac_label_probe"

static void copy_file(const char *source, const char *destination)
{
	int in_fd = CHECK(open(source, O_RDONLY));
	int out_fd = CHECK(open(destination, O_CREAT | O_WRONLY | O_TRUNC, 0700));
	char buffer[4096];

	for (;;) {
		ssize_t read_len = CHECK(read(in_fd, buffer, sizeof(buffer)));
		if (read_len == 0) {
			break;
		}
		CHECK(write(out_fd, buffer, read_len));
	}

	CHECK(close(in_fd));
	CHECK(close(out_fd));
	CHECK(chmod(destination, 0700));
}

static void clear_label_xattr(const char *path)
{
	if (removexattr(path, "security.aster_mac.label") == 0 || errno == ENODATA) {
		return;
	}

	perror("removexattr");
	exit(EXIT_FAILURE);
}

static int read_current_label(char *buffer, size_t buffer_len)
{
	if (buffer_len == 0) {
		errno = EINVAL;
		return -1;
	}

	int fd = open(CURRENT_LABEL_FILE, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t read_len = read(fd, buffer, buffer_len - 1);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;
	if (read_len < 0) {
		return -1;
	}

	buffer[read_len] = '\0';
	buffer[strcspn(buffer, "\n")] = '\0';
	return 0;
}

static int expect_exec_label(const char *expected)
{
	pid_t pid = fork();
	if (pid < 0) {
		return -1;
	}

	if (pid == 0) {
		execl(LABEL_PROBE, LABEL_PROBE, expected, NULL);
		_exit(100);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		return -1;
	}

	if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
		errno = 0;
		return 0;
	}

	errno = EIO;
	return -1;
}

FN_SETUP(prepare_label_probe)
{
	copy_file(LABEL_PROBE_SOURCE, LABEL_PROBE);
	clear_label_xattr(LABEL_PROBE);
}
END_SETUP()

FN_TEST(default_task_label_is_visible)
{
	char label[128];

	TEST_RES(read_current_label(label, sizeof(label)),
		 _ret == 0 && strcmp(label, "unconfined") == 0);
}
END_TEST()

FN_TEST(exec_transition_updates_current_label)
{
	static const char exec_label[] = "domain.exec";

	TEST_SUCC(setxattr(LABEL_PROBE, "security.aster_mac.label", exec_label,
			   strlen(exec_label), 0));
	TEST_SUCC(expect_exec_label(exec_label));
	TEST_SUCC(removexattr(LABEL_PROBE, "security.aster_mac.label"));
	TEST_SUCC(expect_exec_label("unconfined"));
}
END_TEST()
