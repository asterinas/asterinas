// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define SYSLOG_ACTION_CLOSE 0
#define SYSLOG_ACTION_OPEN 1
#define SYSLOG_ACTION_READ 2
#define SYSLOG_ACTION_READ_ALL 3
#define SYSLOG_ACTION_READ_CLEAR 4
#define SYSLOG_ACTION_CONSOLE_LEVEL 8
#define SYSLOG_ACTION_SIZE_UNREAD 9
#define SYSLOG_ACTION_SIZE_BUFFER 10

#define DMESG_RESTRICT_PATH "/proc/sys/kernel/dmesg_restrict"
#define DEFAULT_CONSOLE_LOGLEVEL 4

static int saved_dmesg_restrict;

static long syslog_call(int action, void *buf, long len)
{
	return syscall(SYS_syslog, action, buf, len);
}

static long syslog_action(int action, long len)
{
	return syslog_call(action, NULL, len);
}

static int read_dmesg_restrict(void)
{
	char value;
	int fd = CHECK(open(DMESG_RESTRICT_PATH, O_RDONLY));

	CHECK_WITH(read(fd, &value, 1), _ret == 1);
	CHECK(close(fd));
	return value == '0' ? 0 : 1;
}

static void write_dmesg_restrict(int value)
{
	char text = value ? '1' : '0';
	int fd = CHECK(open(DMESG_RESTRICT_PATH, O_WRONLY));

	CHECK_WITH(write(fd, &text, 1), _ret == 1);
	CHECK(close(fd));
}

FN_SETUP(init)
{
	saved_dmesg_restrict = read_dmesg_restrict();
	write_dmesg_restrict(0);
}
END_SETUP()

FN_TEST(console_level_range)
{
	TEST_ERRNO(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL, 0), EINVAL);
	TEST_SUCC(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL, 1));
	TEST_SUCC(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL, 8));
	TEST_ERRNO(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL, 9), EINVAL);
	TEST_ERRNO(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL, 256), EINVAL);
	TEST_SUCC(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL,
				DEFAULT_CONSOLE_LOGLEVEL));
}
END_TEST()

FN_TEST(read_argument_errors)
{
	char buf;

	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ, NULL, 0), EINVAL);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ_ALL, NULL, 0), EINVAL);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ_ALL, NULL, 1), EINVAL);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ_ALL, &buf, -1), EINVAL);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ_CLEAR, NULL, 0), EINVAL);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_READ_CLEAR, &buf, -1), EINVAL);
	TEST_SUCC(syslog_call(SYSLOG_ACTION_READ_ALL, &buf, 0));
	TEST_SUCC(syslog_call(SYSLOG_ACTION_READ_CLEAR, &buf, 0));
}
END_TEST()

FN_TEST(unprivileged_actions_when_dmesg_is_unrestricted)
{
	int status;
	pid_t child;
	char buf;

	TEST_RES(syslog_action(SYSLOG_ACTION_SIZE_UNREAD, 0), _ret >= 0);
	TEST_RES(syslog_action(SYSLOG_ACTION_SIZE_BUFFER, 0), _ret > 0);

	child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(setresgid(65534, 65534, 65534));
		CHECK(setresuid(65534, 65534, 65534));

		CHECK_WITH(syslog_action(SYSLOG_ACTION_CLOSE, 0),
			   _ret < 0 && errno == EPERM);
		CHECK_WITH(syslog_action(SYSLOG_ACTION_OPEN, 0),
			   _ret < 0 && errno == EPERM);
		CHECK_WITH(syslog_call(SYSLOG_ACTION_READ, &buf, 0),
			   _ret < 0 && errno == EPERM);
		CHECK_WITH(syslog_call(SYSLOG_ACTION_READ_ALL, &buf, 1),
			   _ret >= 0);
		CHECK_WITH(syslog_action(SYSLOG_ACTION_SIZE_UNREAD, 0),
			   _ret < 0 && errno == EPERM);
		CHECK_WITH(syslog_action(SYSLOG_ACTION_SIZE_BUFFER, 0),
			   _ret > 0);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(unprivileged_read_all_when_dmesg_is_restricted)
{
	int status;
	pid_t child;
	char buf;

	write_dmesg_restrict(1);

	child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(setresgid(65534, 65534, 65534));
		CHECK(setresuid(65534, 65534, 65534));

		CHECK_WITH(syslog_call(SYSLOG_ACTION_READ_ALL, &buf, 1),
			   _ret < 0 && errno == EPERM);
		CHECK_WITH(syslog_action(SYSLOG_ACTION_SIZE_BUFFER, 0),
			   _ret < 0 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(child, &status, 0),
		 _ret == child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	write_dmesg_restrict(0);
}
END_TEST()

FN_SETUP(cleanup)
{
	write_dmesg_restrict(saved_dmesg_restrict);
	CHECK(syslog_action(SYSLOG_ACTION_CONSOLE_LEVEL,
			    DEFAULT_CONSOLE_LOGLEVEL));
}
END_SETUP()
