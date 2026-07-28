// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define HOST_SEED EXT_TEST_ROOT "/seed.txt"

FN_TEST(read_host_created_file)
{
	static const char expected[] =
		"This file was created by mke2fs on the host.\n";
	char buffer[sizeof(expected)] = { 0 };

	int fd = TEST_SUCC(open(HOST_SEED, O_RDONLY));
	TEST_RES(read(fd, buffer, sizeof(buffer)),
		 _ret == (ssize_t)(sizeof(expected) - 1));
	TEST_RES(memcmp(buffer, expected, sizeof(expected) - 1), _ret == 0);
	TEST_RES(read(fd, buffer, sizeof(buffer)), _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()
