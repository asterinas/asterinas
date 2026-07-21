// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include "../../common/test.h"

#ifndef FALLOC_FL_KEEP_SIZE
#define FALLOC_FL_KEEP_SIZE 0x01
#endif

#ifndef FALLOC_FL_PUNCH_HOLE
#define FALLOC_FL_PUNCH_HOLE 0x02
#endif

FN_TEST(name_too_long)
{
	char name[251];
	int fd;

	memset(name, 'X', sizeof(name));

	name[250] = '\0';
	TEST_ERRNO(memfd_create(name, MFD_CLOEXEC), EINVAL);

	name[249] = '\0';
	fd = TEST_SUCC(memfd_create(name, MFD_CLOEXEC));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(fallocate_seals)
{
	int fd;

	fd = TEST_SUCC(memfd_create("test_memfd", MFD_ALLOW_SEALING));
	TEST_SUCC(ftruncate(fd, 4096));
	TEST_SUCC(fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE));
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE | FALLOC_FL_PUNCH_HOLE, 0,
			     4096),
		   EPERM);
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(memfd_create("test_memfd", MFD_ALLOW_SEALING));
	TEST_SUCC(ftruncate(fd, 4096));
	TEST_SUCC(fcntl(fd, F_ADD_SEALS, F_SEAL_GROW));
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
	TEST_ERRNO(fallocate(fd, 0, 0, 8192), EPERM);
	TEST_ERRNO(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 8192), EPERM);
	TEST_SUCC(close(fd));
}
END_TEST()
