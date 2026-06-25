// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(fionbio_fioasync_null_arg)
{
	int fd;

	fd = TEST_SUCC(open("/dev/null", O_RDONLY));
	TEST_ERRNO(ioctl(fd, FIONBIO, NULL), EFAULT);
	TEST_ERRNO(ioctl(fd, FIOASYNC, NULL), EFAULT);
	TEST_SUCC(close(fd));
}
END_TEST()
