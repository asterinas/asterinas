// SPDX-License-Identifier: MPL-2.0

#include <sys/mman.h>
#include <sys/fcntl.h>
#include <unistd.h>

#include "../test.h"

#define PAGE_SIZE 4096

FN_TEST(mmap_beyond_the_file)
{
	const char *filename = "mmap_test_file";
	int fd = TEST_SUCC(open(filename, O_CREAT | O_RDWR, 0600));
	TEST_SUCC(ftruncate(fd, 2 * PAGE_SIZE));

	char *addr = CHECK_WITH(mmap(NULL, 4 * PAGE_SIZE,
				     PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0),
				_ret != MAP_FAILED);
	TEST_RES(addr[1 * PAGE_SIZE], _ret == 0);
	// The following operation (if uncommented) would cause a segmentation fault.
	// TEST_RES(addr[3 * PAGE_SIZE], _ret == 0);

	TEST_SUCC(munmap(addr, 4 * PAGE_SIZE));

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(filename));
}
END_TEST()
