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
	TEST_ERRNO(mremap(valid_addr, ~(size_t)1, PAGE_SIZE, 0), EINVAL);
	TEST_ERRNO(mremap(valid_addr, ~(size_t)1, PAGE_SIZE, MREMAP_MAYMOVE),
		   EINVAL);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, ~(size_t)1, 0), EINVAL);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, ~(size_t)1, MREMAP_MAYMOVE),
		   EINVAL);
	TEST_ERRNO(munmap(valid_addr, ~(size_t)1), EINVAL);
	TEST_ERRNO(mprotect(valid_addr, ~(size_t)1, PROT_READ), ENOMEM);
	TEST_ERRNO(madvise(valid_addr, ~(size_t)1, MADV_NORMAL), EINVAL);
	// FIXME: Linux will "align up" `~(size_t)1` to zero, and `msync` will succeed
	// if the length is zero. This is probably a bug rather than a feature.
#ifdef __asterinas__
	TEST_ERRNO(msync(valid_addr, ~(size_t)1, 0), ENOMEM);
#else
	TEST_SUCC(msync(valid_addr, ~(size_t)1, 0));
#endif
}
END_TEST()

FN_TEST(zero_len)
{
	TEST_ERRNO(mmap(valid_addr, 0, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS,
			0, 0),
		   EINVAL);
	TEST_ERRNO(mremap(valid_addr, 0, PAGE_SIZE, 0), EINVAL);
	TEST_ERRNO(mremap(valid_addr, 0, PAGE_SIZE, MREMAP_MAYMOVE), EINVAL);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, 0, 0), EINVAL);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, 0, MREMAP_MAYMOVE), EINVAL);
	TEST_ERRNO(munmap(valid_addr, 0), EINVAL);
	TEST_SUCC(mprotect(valid_addr, 0, PROT_READ));
	TEST_SUCC(madvise(valid_addr, 0, MADV_NORMAL));
	TEST_SUCC(msync(valid_addr, 0, 0));
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
		TEST_ERRNO(mremap(valid_addr, len, PAGE_SIZE, 0), EINVAL);
		TEST_ERRNO(mremap(valid_addr, len, PAGE_SIZE, MREMAP_MAYMOVE),
			   EINVAL);
		TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, len, 0), EINVAL);
		// FIXME: Asterinas will return `ENOMEM` in this test, which differs
		// from Linux's error code.
#ifdef __asterinas__
		TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, len, MREMAP_MAYMOVE),
			   ENOMEM);
#else
		TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, len, MREMAP_MAYMOVE),
			   EINVAL);
#endif
		TEST_ERRNO(munmap(valid_addr, len), EINVAL);
		TEST_ERRNO(mprotect(valid_addr, len, PROT_READ), ENOMEM);
		TEST_ERRNO(madvise(valid_addr, len, MADV_NORMAL), EINVAL);
		TEST_ERRNO(msync(valid_addr, len, 0), ENOMEM);
	}
}
END_TEST()

FN_TEST(underflow_addr)
{
	void *addr = (void *)PAGE_SIZE;

	TEST_ERRNO(mmap(addr, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, 0, 0),
		   EPERM);
	TEST_ERRNO(mremap(addr, PAGE_SIZE, PAGE_SIZE, 0), EFAULT);
	TEST_ERRNO(mremap(addr, PAGE_SIZE, PAGE_SIZE, MREMAP_MAYMOVE), EFAULT);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, PAGE_SIZE, MREMAP_FIXED, addr),
		   EINVAL);
	TEST_SUCC(munmap(addr, PAGE_SIZE));
	TEST_ERRNO(mprotect(addr, PAGE_SIZE, PROT_READ), ENOMEM);
	TEST_ERRNO(madvise(addr, PAGE_SIZE, MADV_NORMAL), ENOMEM);
	TEST_ERRNO(msync(addr, PAGE_SIZE, 0), ENOMEM);
}
END_TEST()

FN_TEST(unaligned_addr)
{
	TEST_ERRNO(mmap(valid_addr + 1, PAGE_SIZE, PROT_READ,
			MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, 0, 0),
		   EINVAL);
	TEST_ERRNO(mremap(valid_addr + 1, PAGE_SIZE, PAGE_SIZE, 0), EINVAL);
	TEST_ERRNO(mremap(valid_addr + 1, PAGE_SIZE, PAGE_SIZE, MREMAP_MAYMOVE),
		   EINVAL);
	TEST_ERRNO(mremap(valid_addr, PAGE_SIZE, PAGE_SIZE, MREMAP_FIXED,
			  valid_addr + 1),
		   EINVAL);
	TEST_ERRNO(munmap(valid_addr + 1, PAGE_SIZE), EINVAL);
	TEST_ERRNO(mprotect(valid_addr + 1, PAGE_SIZE, PROT_READ), EINVAL);
	TEST_ERRNO(madvise(valid_addr + 1, PAGE_SIZE, MADV_NORMAL), EINVAL);
	TEST_ERRNO(msync(valid_addr + 1, PAGE_SIZE, 0), EINVAL);
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
	TEST_ERRNO(mmap(NULL, PAGE_SIZE, PROT_READ,
			MAP_SHARED_VALIDATE | MAP_SYNC, fd, 0),
		   EOPNOTSUPP);
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
