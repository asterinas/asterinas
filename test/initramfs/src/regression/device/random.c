// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/mman.h>
#include <sys/fcntl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include "../common/test.h"

#define PAGE_SIZE 4096

struct random_device {
	const char *path;
	unsigned int major;
	unsigned int minor;
};

static const struct random_device random_devices[] = {
	{ "/dev/random", 1, 8 },
	{ "/dev/urandom", 1, 9 },
};

FN_TEST(rdev_and_mode)
{
	for (size_t i = 0;
	     i < sizeof(random_devices) / sizeof(random_devices[0]); i++) {
		const struct random_device *device = &random_devices[i];
		struct stat stat_buf;

		TEST_RES(stat(device->path, &stat_buf),
			 S_ISCHR(stat_buf.st_mode) &&
				 stat_buf.st_rdev == makedev(device->major,
							     device->minor) &&
				 (stat_buf.st_mode & 0777) == 0666);
	}
}
END_TEST()

FN_TEST(short_rw)
{
	int fd;
	char *buf;

	fd = TEST_SUCC(open("/dev/random", O_RDONLY));

	buf = TEST_SUCC(mmap(NULL, PAGE_SIZE * 3, PROT_READ | PROT_WRITE,
			     MAP_ANONYMOUS | MAP_PRIVATE, -1, 0));
	TEST_SUCC(munmap(buf + PAGE_SIZE * 2, PAGE_SIZE));

	// Invalid address
	TEST_ERRNO(read(fd, buf + PAGE_SIZE * 2, PAGE_SIZE), EFAULT);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2, 0), _ret == 0);

	// Valid address, insufficient space
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - 1, PAGE_SIZE), _ret == 1);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE - 1), PAGE_SIZE + 2),
		 _ret == (PAGE_SIZE - 1));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - PAGE_SIZE, PAGE_SIZE + 2),
		 _ret == PAGE_SIZE);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE + 1), PAGE_SIZE + 2),
		 _ret == (PAGE_SIZE + 1));

	// Valid address, sufficient space
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - 1, 1), _ret == 1);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE - 1), PAGE_SIZE - 2),
		 _ret == (PAGE_SIZE - 2));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - PAGE_SIZE, PAGE_SIZE - 1),
		 _ret == (PAGE_SIZE - 1));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE + 1), PAGE_SIZE),
		 _ret == PAGE_SIZE);

	TEST_SUCC(munmap(buf, PAGE_SIZE * 2));
	TEST_SUCC(close(fd));
}
END_TEST()
