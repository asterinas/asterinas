// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"

#include <fcntl.h>
#include <unistd.h>

FN_TEST(sysfs_devices_lseek)
{
	int fd = TEST_RES(open("/sys/devices", O_RDONLY), _ret >= 0);
	if (fd < 0)
		goto out;

	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 0);
	TEST_RES(lseek(fd, 0, SEEK_END), _ret == 0);

	TEST_SUCC(close(fd));
out:;
}
END_TEST()
