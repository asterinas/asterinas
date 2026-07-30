// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <fcntl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static const char *POLICY_VERSION_PATH =
	"/sys/kernel/security/apparmor/features/policy_version";
static const char *POLICY_LOAD_PATH = "/sys/kernel/security/apparmor/.load";
static const char *PROFILES_PATH = "/sys/kernel/security/apparmor/profiles";
static const char *CURRENT_ATTR_PATH = "/proc/self/attr/current";
static const char *HELPER_PATH = "/test/security/lsm/apparmor_exec_helper";
static const char *ALLOWED_PATH = "/tmp/apparmor-file-open-allowed";
static const char *DENIED_PATH = "/tmp/apparmor-file-open-denied";
static const char *DENIED_CREATE_PATH = "/tmp/apparmor-file-open-denied-create";
static const char *UNMEDIATED_FIFO_PATH =
	"/tmp/apparmor-file-open-unmediated-fifo";
static const char *ALLOWED_CONTENT = "allowed\n";
static const char *DENIED_CONTENT = "must survive truncation\n";

enum { PROFILE_NAME_LEN = 128 };
static char profile_name[PROFILE_NAME_LEN + 1];

static void write_whole_file(const char *path, const char *content)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600));
	size_t length = strlen(content);

	CHECK_WITH(write(fd, content, length), _ret == (ssize_t)length);
	CHECK(close(fd));
}

static void read_whole_file(const char *path, char *buffer, size_t buffer_size)
{
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t length = CHECK(read(fd, buffer, buffer_size - 1));

	buffer[length] = '\0';
	CHECK(close(fd));
}

static void write_profile(const char *control_path)
{
	char policy[512];
	int length = CHECK(snprintf(policy, sizeof(policy),
				    "version 0\n"
				    "profile %s\n"
				    "%s r\n",
				    profile_name, ALLOWED_PATH));
	int fd = CHECK(open(control_path, O_WRONLY));

	CHECK_WITH(write(fd, policy, length), _ret == (ssize_t)length);
	CHECK(close(fd));
}

static void run_confined_helper(void)
{
	char current_value[PROFILE_NAME_LEN + 2];
	int fd = CHECK(open(CURRENT_ATTR_PATH, O_WRONLY));
	int length = CHECK(snprintf(current_value, sizeof(current_value),
				    "%s\n", profile_name));

	CHECK_WITH(write(fd, current_value, length), _ret == (ssize_t)length);
	CHECK(close(fd));
	execl(HELPER_PATH, HELPER_PATH, NULL);
	_exit(120);
}

static void reject_too_long_profile_name(void)
{
	char too_long_name[PROFILE_NAME_LEN + 1];
	int fd = CHECK(open(CURRENT_ATTR_PATH, O_WRONLY));

	memset(too_long_name, 'b', sizeof(too_long_name));
	CHECK_WITH(write(fd, too_long_name, sizeof(too_long_name)),
		   _ret == -1 && errno == EINVAL);
	CHECK(close(fd));
}

FN_TEST(enforces_file_open_across_exec)
{
	char buffer[256] = { 0 };
	struct stat statbuf;

	TEST_SUCC(access(POLICY_VERSION_PATH, F_OK));
	memset(profile_name, 'a', PROFILE_NAME_LEN);
	profile_name[PROFILE_NAME_LEN] = '\0';

	CHECK_WITH(unlink(DENIED_CREATE_PATH), _ret == 0 || errno == ENOENT);
	CHECK_WITH(unlink(UNMEDIATED_FIFO_PATH), _ret == 0 || errno == ENOENT);
	TEST_SUCC(mkfifo(UNMEDIATED_FIFO_PATH, 0600));
	write_whole_file(ALLOWED_PATH, ALLOWED_CONTENT);
	write_whole_file(DENIED_PATH, DENIED_CONTENT);
	write_profile(POLICY_LOAD_PATH);
	reject_too_long_profile_name();

	read_whole_file(CURRENT_ATTR_PATH, buffer, sizeof(buffer));
	TEST_RES(strcmp(buffer, "unconfined\n"), _ret == 0);
	memset(buffer, 0, sizeof(buffer));
	read_whole_file(PROFILES_PATH, buffer, sizeof(buffer));
	TEST_RES(strstr(buffer, profile_name), _ret != NULL);

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		run_confined_helper();
	}

	int status = 0;
	TEST_RES(waitpid(child, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);

	memset(buffer, 0, sizeof(buffer));
	read_whole_file(CURRENT_ATTR_PATH, buffer, sizeof(buffer));
	TEST_RES(strcmp(buffer, "unconfined\n"), _ret == 0);
	memset(buffer, 0, sizeof(buffer));
	read_whole_file(DENIED_PATH, buffer, sizeof(buffer));
	TEST_RES(strcmp(buffer, DENIED_CONTENT), _ret == 0);
	TEST_RES(stat(DENIED_CREATE_PATH, &statbuf), S_ISREG(statbuf.st_mode));

	TEST_SUCC(unlink(ALLOWED_PATH));
	TEST_SUCC(unlink(DENIED_PATH));
	TEST_SUCC(unlink(DENIED_CREATE_PATH));
	TEST_SUCC(unlink(UNMEDIATED_FIFO_PATH));
}
END_TEST()
