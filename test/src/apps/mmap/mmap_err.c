// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/mman.h>
#include <sys/fcntl.h>
#include <unistd.h>

#include "../test.h"

#define PAGE_SIZE 4096

static void *valid_addr;
static void *avail_addr;
static int fd;

FN_SETUP(init)
{
	valid_addr = CHECK_WITH(mmap(NULL, PAGE_SIZE * 2, PROT_READ,
				     MAP_PRIVATE | MAP_ANONYMOUS, 0, 0),
				_ret != MAP_FAILED);

	avail_addr = valid_addr + PAGE_SIZE;
	CHECK(munmap(avail_addr, PAGE_SIZE));

	fd = CHECK(open("/proc/self/exe", O_RDONLY));
}
END_SETUP()

FN_TEST(overflow_len)
{
	TEST_ERRNO(mmap(valid_addr, ~(size_t)1, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS, 0, 0),
		   ENOMEM);
}
END_TEST()

FN_TEST(zero_len)
{
	TEST_ERRNO(mmap(valid_addr, 0, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS,
			0, 0),
		   EINVAL);
}
END_TEST()

FN_TEST(overflow_addr)
{
	size_t exact_len = -(size_t)valid_addr;

	for (int diff = -1; diff <= 1; ++diff) {
		size_t len = exact_len + diff;

		TEST_ERRNO(mmap(valid_addr, len, PROT_READ,
				MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, 0, 0),
			   ENOMEM);
	}
}
END_TEST()

FN_TEST(underflow_addr)
{
	void *addr = (void *)PAGE_SIZE;

	TEST_ERRNO(mmap(addr, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, 0, 0),
		   EPERM);
}
END_TEST()

FN_TEST(unaligned_addr)
{
	TEST_ERRNO(mmap(valid_addr + 1, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, 0, 0),
		   EINVAL);
}
END_TEST()

FN_TEST(overflow_offset)
{
	size_t offset = -(size_t)PAGE_SIZE - PAGE_SIZE;
	int i;
	void *addr;

	for (i = -1; i <= 1; ++i) {
		TEST_ERRNO(mmap(valid_addr, PAGE_SIZE + i, PROT_READ,
				MAP_PRIVATE, fd, offset),
			   EOVERFLOW);
		TEST_ERRNO(mmap(valid_addr, PAGE_SIZE * 2 + i, PROT_READ,
				MAP_PRIVATE, fd, offset),
			   EOVERFLOW);

		addr = TEST_SUCC(mmap(valid_addr, PAGE_SIZE + i, PROT_READ,
				      MAP_PRIVATE | MAP_ANONYMOUS, fd, offset));
		TEST_SUCC(munmap(addr, PAGE_SIZE + i));
		addr = TEST_SUCC(mmap(valid_addr, PAGE_SIZE * 2 + i, PROT_READ,
				      MAP_PRIVATE | MAP_ANONYMOUS, fd, offset));
		TEST_SUCC(munmap(addr, PAGE_SIZE * 2 + i));
	}
}
END_TEST()

FN_TEST(unaligned_offset)
{
	TEST_ERRNO(mmap(valid_addr, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS, fd, 1),
		   EINVAL);
	TEST_ERRNO(mmap(valid_addr, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS, fd, 2048),
		   EINVAL);
}
END_TEST()

FN_TEST(mmap_flags)
{
	void *addr;

	addr = TEST_SUCC(
		mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SHARED | MAP_SYNC, fd, 0));
	TEST_SUCC(munmap(addr, PAGE_SIZE));

	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ, MAP_SYNC, fd, 0), EINVAL);
	TEST_ERRNO(mmap(avail_addr, PAGE_SIZE, PROT_READ,
			MAP_SHARED_VALIDATE | MAP_FIXED_NOREPLACE, fd, 0),
		   EOPNOTSUPP);
	TEST_ERRNO(mmap(valid_addr, PAGE_SIZE, PROT_READ,
			MAP_SHARED | MAP_FIXED_NOREPLACE, fd, 0),
		   EEXIST);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(munmap(valid_addr, PAGE_SIZE));

	CHECK(close(fd));
}
END_SETUP()
