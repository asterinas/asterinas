// SPDX-License-Identifier: MPL-2.0

#include <sys/mman.h>
#include <sys/fcntl.h>
#include <unistd.h>

#include "../network/test.h"

#define FILE_NAME "/tmp/mmap_readahead.txt"

#define PAGE_SIZE 4096
#define NR_PAGES 16

static char *addr;

FN_SETUP(mmap_readahead)
{
	int fd;

	fd = CHECK(open(FILE_NAME, O_RDWR | O_CREAT));
	CHECK(unlink(FILE_NAME));

	CHECK(ftruncate(fd, PAGE_SIZE * NR_PAGES));

	addr = mmap(NULL, PAGE_SIZE * NR_PAGES, PROT_READ | PROT_WRITE,
		    MAP_SHARED, fd, 0);
	CHECK(addr == MAP_FAILED ? -1 : 0);
}
END_SETUP()

FN_TEST(mmap_readahead)
{
	int i;

	for (i = 0; i < NR_PAGES; ++i) {
		TEST_RES(addr[i * PAGE_SIZE], _ret == 0);
	}
}
END_TEST()
