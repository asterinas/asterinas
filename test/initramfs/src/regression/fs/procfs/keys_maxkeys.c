// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

#include "../../common/test.h"

#define MAXKEYS_PATH "/proc/sys/kernel/keys/maxkeys"

static long read_maxkeys(void)
{
	char buf[64] = { 0 };
	char *end;
	int fd = open(MAXKEYS_PATH, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t bytes_read = read(fd, buf, sizeof(buf) - 1);
	int saved_errno = errno;
	if (close(fd) < 0) {
		return -1;
	}

	if (bytes_read <= 0) {
		errno = bytes_read < 0 ? saved_errno : EINVAL;
		return -1;
	}
	if (buf[bytes_read - 1] != '\n') {
		errno = EINVAL;
		return -1;
	}

	errno = 0;
	long value = strtol(buf, &end, 10);
	if (errno != 0) {
		return -1;
	}
	if (end <= buf || *end != '\n') {
		errno = EINVAL;
		return -1;
	}

	errno = 0;
	return value;
}

FN_TEST(proc_sys_kernel_keys_maxkeys)
{
	TEST_RES(read_maxkeys(), _ret == 200);

	int fd = TEST_SUCC(open(MAXKEYS_PATH, O_WRONLY));
	TEST_ERRNO(write(fd, "-1", 2), EINVAL);
	TEST_SUCC(close(fd));
	TEST_RES(read_maxkeys(), _ret == 200);

	fd = TEST_SUCC(open(MAXKEYS_PATH, O_WRONLY));
	TEST_RES(write(fd, "100", 3), _ret == 3);
	TEST_SUCC(close(fd));
	TEST_RES(read_maxkeys(), _ret == 100);

	fd = TEST_SUCC(open(MAXKEYS_PATH, O_WRONLY));
	TEST_RES(write(fd, "200", 3), _ret == 3);
	TEST_SUCC(close(fd));
	TEST_RES(read_maxkeys(), _ret == 200);
}
END_TEST()
