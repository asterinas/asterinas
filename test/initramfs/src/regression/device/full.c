// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/poll.h>
#include "../common/test.h"

#define DEVICE_PATH "/dev/full"
#define READ_SIZE 100

int fd;
char buffer[READ_SIZE];

FN_SETUP(open)
{
	fd = CHECK(open(DEVICE_PATH, O_RDWR));
}
END_SETUP()

FN_TEST(fstat)
{
	struct stat stat;
	TEST_RES(fstat(fd, &stat),
		 S_ISCHR(stat.st_mode) && stat.st_rdev == makedev(0x1, 0x7));
}
END_TEST()

FN_TEST(read)
{
	memset(buffer, 1, sizeof(buffer));
	char all_zeros[READ_SIZE] = { 0 };

	TEST_RES(read(fd, buffer, READ_SIZE),
		 _ret == READ_SIZE &&
			 memcmp(buffer, all_zeros, READ_SIZE) == 0);
	TEST_RES(read(fd, buffer, 0), _ret == 0);

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wnonnull"
	TEST_ERRNO(read(fd, NULL, 1), EFAULT);
	TEST_RES(read(fd, NULL, 0), _ret == 0);
#pragma GCC diagnostic pop
}
END_TEST()

FN_TEST(write)
{
	TEST_ERRNO(write(fd, buffer, 1), ENOSPC);
	TEST_ERRNO(write(fd, buffer, 0), ENOSPC);

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wnonnull"
	TEST_ERRNO(write(fd, NULL, 1), ENOSPC);
	TEST_ERRNO(write(fd, NULL, 0), ENOSPC);
#pragma GCC diagnostic pop
}
END_TEST()

FN_TEST(poll)
{
	struct pollfd pfd = { .fd = fd, .events = POLLIN | POLLOUT };
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
}
END_TEST()

FN_SETUP(close)
{
	CHECK(close(fd));
}
END_SETUP()
