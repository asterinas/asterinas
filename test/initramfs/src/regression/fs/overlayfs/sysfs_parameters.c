// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(overlay_parameters)
{
	const char *params[] = {
		"/sys/module/overlay/parameters/index",
		"/sys/module/overlay/parameters/redirect_dir",
		"/sys/module/overlay/parameters/metacopy",
		"/sys/module/overlay/parameters/nfs_export",
		"/sys/module/overlay/parameters/xino_auto",
	};

	for (size_t i = 0; i < sizeof(params) / sizeof(params[0]); i++) {
		const char *path = params[i];
		char buf[32] = { 0 };

		int fd = open(path, O_RDONLY);
		TEST_RES(fd, _ret >= 0);
		if (fd >= 0) {
			ssize_t n = read(fd, buf, sizeof(buf) - 1);
			TEST_RES(n, _ret == 2);
			TEST_RES(strcmp(buf, "N\n"), _ret == 0);
			close(fd);
		}

		int wfd = open(path, O_WRONLY);
		if (wfd >= 0) {
			TEST_ERRNO(write(wfd, "Y\n", 2), EIO);
			close(wfd);
		} else {
			TEST_RES(errno, _ret == EACCES);
		}
	}

	TEST_ERRNO(open("/sys/module/overlay/parameters/nonexistent", O_RDONLY), ENOENT);
}

END_TEST()
