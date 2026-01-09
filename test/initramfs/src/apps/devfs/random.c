// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/mman.h>
#include <sys/fcntl.h>
#include "../test.h"

#define PAGE_SIZE 4096

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
