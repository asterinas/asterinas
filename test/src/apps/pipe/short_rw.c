// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/mman.h>
#include <stdio.h>

#include "../test.h"

#define PAGE_SIZE 4096

static void *page;
static int rfd, wfd;

FN_SETUP(short_read_and_write)
{
	int fildes[2];

	page = mmap((void *)0x20000000, PAGE_SIZE, PROT_READ | PROT_WRITE,
		    MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
	CHECK(page == NULL ? -1 : 0);

	CHECK(pipe(fildes));
	rfd = fildes[0];
	wfd = fildes[1];

	CHECK_WITH(write(wfd, "ab", 2), _ret == 2);
}
END_SETUP()

FN_TEST(short_read_and_write)
{
	char *buf = page + PAGE_SIZE - 1;
	buf[0] = 'x';

	TEST_ERRNO(read(rfd, buf, 2), EFAULT);

	TEST_RES(read(rfd, buf, 1), _ret == 1 && buf[0] == 'a');

	TEST_ERRNO(write(wfd, buf, 2), EFAULT);

	TEST_RES(write(wfd, buf, 1), _ret == 1);
}
END_TEST()