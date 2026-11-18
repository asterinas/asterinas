// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stdint.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

/* Run on opposite sides of a real unmount with bsddf/minixdf options. */
FN_TEST(statfs_respects_block_accounting_option)
{
	const char *device = getenv("EXT_STATFS_DEVICE");
	const char *path = getenv("EXT_STATFS_PATH");
	const char *expected = getenv("EXT_STATFS_EXPECT");
	CHECK_WITH(device && path && expected, _ret);
	int fd = TEST_SUCC(open(device, O_RDONLY));
	unsigned char bytes[4];
	/* s_blocks_count_lo in the ext superblock, independently of statfs. */
	TEST_RES(pread(fd, bytes, sizeof(bytes), 1024 + 4),
		 _ret == sizeof(bytes));
	TEST_SUCC(close(fd));
	uint32_t total = (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
			 (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
	struct statfs st;
	TEST_SUCC(statfs(path, &st));
	if (!strcmp(expected, "total")) {
		TEST_RES(st.f_blocks, _ret == total);
	} else if (!strcmp(expected, "overhead")) {
		TEST_RES(st.f_blocks, _ret > 0 && _ret < total);
	} else {
		TEST_RES(st.f_blocks, _ret == strtoul(expected, NULL, 10));
	}
}
END_TEST()
