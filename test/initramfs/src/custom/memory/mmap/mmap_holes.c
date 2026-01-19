// SPDX-License-Identifier: MPL-2.0

#include <sys/mman.h>

#include "../../common/test.h"

static char *start_addr;

#define PAGE_SIZE 4096

FN_SETUP(init)
{
	start_addr = CHECK_WITH(mmap(NULL, PAGE_SIZE * 4, PROT_READ,
				     MAP_PRIVATE | MAP_ANONYMOUS, 0, 0),
				_ret != MAP_FAILED);

	CHECK(munmap(start_addr + PAGE_SIZE * 2, PAGE_SIZE));
}
END_SETUP()

FN_TEST(mprotect)
{
	// `mprotect` takes effect only for pages before the hole.
	TEST_ERRNO(mprotect(start_addr + PAGE_SIZE, PAGE_SIZE * 3,
			    PROT_READ | PROT_WRITE),
		   ENOMEM);
	TEST_RES(start_addr[PAGE_SIZE] = 12, start_addr[PAGE_SIZE] == 12);

	TEST_ERRNO(mprotect(start_addr + PAGE_SIZE * 2, PAGE_SIZE * 2,
			    PROT_READ | PROT_WRITE),
		   ENOMEM);
	TEST_SUCC(mprotect(start_addr + PAGE_SIZE * 3, PAGE_SIZE,
			   PROT_READ | PROT_WRITE));
	TEST_RES(start_addr[PAGE_SIZE * 3] = 45,
		 start_addr[PAGE_SIZE * 3] == 45);
}
END_TEST()

FN_TEST(madvise)
{
	// `madvise` takes effect for pages before and after the hole.
	TEST_ERRNO(madvise(start_addr + PAGE_SIZE, PAGE_SIZE * 3,
			   MADV_DONTNEED),
		   ENOMEM);

	TEST_RES(start_addr[PAGE_SIZE], _ret == 0);
	TEST_RES(start_addr[PAGE_SIZE * 3], _ret == 0);
}
END_TEST()

FN_TEST(msync)
{
	TEST_ERRNO(msync(start_addr + PAGE_SIZE, PAGE_SIZE * 3, 0), ENOMEM);
	TEST_ERRNO(msync(start_addr + PAGE_SIZE * 2, PAGE_SIZE * 2, 0), ENOMEM);
	TEST_SUCC(msync(start_addr + PAGE_SIZE * 3, PAGE_SIZE, 0));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(munmap(start_addr, PAGE_SIZE * 4));
}
END_SETUP()
