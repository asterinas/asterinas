// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define TEST_FILE EXT_TEST_ROOT "/fallocate_unsupported"

FN_TEST(fallocate_is_unsupported)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_RDWR, 0644));
	/* FIXME: Linux EXT4 supports this allocation. Asterinas currently
	 * rejects fallocate on extent inodes. Remove the conditional once
	 * extent preallocation is exposed through the VFS.
	 */
#ifdef __asterinas__
	TEST_ERRNO(fallocate(fd, 0, 0, 4096), EOPNOTSUPP);
#else
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
#endif
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()
