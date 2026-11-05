// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

/* Prepared on the ext2 image before running the regression suite. */
#define TARGET "/ext2/open_append_only_target"

FN_TEST(append_only_open_flags)
{
	int fd;

	SKIP_TEST_IF(access(TARGET, F_OK) < 0 && errno == ENOENT);
	fd = TEST(open(TARGET, O_WRONLY), EPERM, _ret == -1);
	if (fd >= 0)
		CHECK(close(fd));
	fd = TEST(open(TARGET, O_RDWR), EPERM, _ret == -1);
	if (fd >= 0)
		CHECK(close(fd));
	fd = TEST(open(TARGET, O_WRONLY | O_TRUNC), EPERM, _ret == -1);
	if (fd >= 0)
		CHECK(close(fd));
	fd = TEST_RES(open(TARGET, O_RDONLY), _ret >= 0);
	if (fd >= 0)
		CHECK(close(fd));
	fd = TEST_RES(open(TARGET, O_WRONLY | O_APPEND), _ret >= 0);
	if (fd >= 0)
		CHECK(close(fd));
}
END_TEST()
