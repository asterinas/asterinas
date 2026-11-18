// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

#define ROOT "/ext-space"
#define BLOCK_SIZE 4096

/* Only the dedicated, 8 MiB EXT2 regression image may be filled. */
FN_TEST(fallocate_enospc_preserves_size_and_free_counts)
{
	struct statfs before, full, after;
	TEST_SUCC(statfs(ROOT, &before));
	CHECK_WITH(before.f_bsize == BLOCK_SIZE && before.f_blocks == 2035,
		   _ret);
	int fd = TEST_SUCC(
		open(ROOT "/target", O_CREAT | O_EXCL | O_RDWR, 0600));
	int filler = TEST_SUCC(
		open(ROOT "/filler", O_CREAT | O_EXCL | O_RDWR, 0600));
	TEST_SUCC(fallocate(fd, 0, 0, 2 * BLOCK_SIZE));
	off_t offset = 0;
	int result, allocation_error;
	/* One-block requests keep failure deterministic without a huge buffer. */
	do {
		errno = 0;
		result = fallocate(filler, 0, offset, BLOCK_SIZE);
		allocation_error = errno;
		if (result == 0)
			offset += BLOCK_SIZE;
		CHECK_WITH(offset <= 2048 * BLOCK_SIZE, _ret);
	} while (result == 0);
	TEST_RES(allocation_error, _ret == ENOSPC);
	TEST_SUCC(statfs(ROOT, &full));
	/* An indirect-node transition may leave one block; use a direct slot. */
	off_t size = 2 * BLOCK_SIZE;
	while (full.f_bfree) {
		CHECK_WITH(size < 12 * BLOCK_SIZE, _ret);
		TEST_SUCC(fallocate(fd, 0, size, BLOCK_SIZE));
		size += BLOCK_SIZE;
		TEST_SUCC(statfs(ROOT, &full));
	}
	struct stat st;
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(st.st_size, _ret == size);
	TEST_ERRNO(fallocate(fd, 0, size, BLOCK_SIZE), ENOSPC);
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(st.st_size, _ret == size);
	TEST_SUCC(statfs(ROOT, &after));
	TEST_RES(after.f_bfree, _ret == full.f_bfree);
	TEST_RES(after.f_ffree, _ret == full.f_ffree);
	TEST_SUCC(close(filler));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(ROOT "/filler"));
	TEST_SUCC(unlink(ROOT "/target"));
	TEST_SUCC(statfs(ROOT, &after));
	TEST_RES(after.f_bfree, _ret == before.f_bfree);
	TEST_RES(after.f_ffree, _ret == before.f_ffree);
}
END_TEST()
