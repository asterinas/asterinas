// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(partial_copy_faults)
{
	int fd = TEST_SUCC(open("/proc/devices", O_RDONLY));
	long page_size = sysconf(_SC_PAGESIZE);
	TEST_RES(page_size, _ret > 0);
	char *mapping = mmap(NULL, 2 * page_size, PROT_READ | PROT_WRITE,
			     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	TEST_RES(mapping != MAP_FAILED, _ret == 1);
	TEST_SUCC(mprotect(mapping + page_size, page_size, PROT_READ));

	char expected[128];
	ssize_t length = TEST_SUCC(pread(fd, expected, sizeof(expected), 0));
	TEST_RES(length, _ret > 32);

	// Seven bytes fault inside the first formatted write; 20 crosses writes.
	for (size_t available = 7; available <= 20; available += 13) {
		for (off_t offset = 0; offset <= 3; offset += 3) {
			char *buffer = mapping + page_size - available;
			TEST_RES(pread(fd, buffer, 32, offset),
				 _ret == (ssize_t)available);
			TEST_RES(memcmp(buffer, expected + offset, available),
				 _ret == 0);
			TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 0);
		}
	}

	TEST_RES(read(fd, mapping + page_size - 7, 32), _ret == 7);
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 7);
	TEST_ERRNO(read(fd, mapping + page_size, 32), EFAULT);
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 7);
	TEST_ERRNO(pread(fd, mapping + page_size, 32, 3), EFAULT);
	TEST_RES(pread(fd, mapping + page_size, 0, 0), _ret == 0);
	TEST_SUCC(munmap(mapping, 2 * page_size));
	TEST_SUCC(close(fd));
}
END_TEST()
