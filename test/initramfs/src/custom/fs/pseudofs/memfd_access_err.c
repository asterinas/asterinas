// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <linux/memfd.h>
#include <sys/ioctl.h>
#include <linux/fs.h>

#include "../../common/test.h"

char memfd_path[64];

FN_SETUP(create)
{
	int fd = CHECK(memfd_create("test_memfd", MFD_ALLOW_SEALING));
	CHECK(ftruncate(fd, 4096));
	CHECK_WITH(snprintf(memfd_path, sizeof(memfd_path), "/proc/self/fd/%d",
			    fd),
		   _ret > 0 && _ret < sizeof(memfd_path));
}
END_SETUP()

FN_TEST(path)
{
	int fd = TEST_SUCC(open(memfd_path, O_PATH | O_RDWR));
	char buf[10];

	TEST_RES(fcntl(fd, F_GETFL), (_ret & O_ACCMODE) == O_RDONLY);

	TEST_ERRNO(fcntl(fd, F_ADD_SEALS, F_SEAL_SEAL), EBADF);
	TEST_ERRNO(fcntl(fd, F_GET_SEALS), EBADF);
	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(ftruncate(fd, 0), EBADF);
	TEST_ERRNO(lseek(fd, 0, SEEK_SET), EBADF);
	TEST_ERRNO(fallocate(fd, 0, 0, 100), EBADF);
	TEST_ERRNO(ioctl(fd, TCGETS), EBADF);
	TEST_ERRNO(mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0), EBADF);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(readonly)
{
	int fd = TEST_SUCC(open(memfd_path, O_RDONLY));
	char buf[10];

	TEST_ERRNO(fcntl(fd, F_ADD_SEALS, F_SEAL_SEAL), EPERM);
	TEST_RES(fcntl(fd, F_GET_SEALS), _ret == 0);
	TEST_SUCC(read(fd, buf, sizeof(buf)));
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_ERRNO(ftruncate(fd, 0), EINVAL);
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_ERRNO(fallocate(fd, 0, 0, 100), EBADF);
	TEST_ERRNO(ioctl(fd, TCGETS), ENOTTY);
	TEST_SUCC(mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(writeonly)
{
	int fd = TEST_SUCC(open(memfd_path, O_WRONLY));
	char buf[10];

	TEST_SUCC(fcntl(fd, F_ADD_SEALS, F_SEAL_SEAL));
	TEST_RES(fcntl(fd, F_GET_SEALS), _ret == F_SEAL_SEAL);
	TEST_ERRNO(read(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(write(fd, buf, sizeof(buf)));
	TEST_SUCC(ftruncate(fd, 0));
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_SUCC(fallocate(fd, 0, 0, 100));
	TEST_ERRNO(ioctl(fd, TCGETS), ENOTTY);
	TEST_ERRNO(mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0), EACCES);

	TEST_SUCC(close(fd));
}
END_TEST()
