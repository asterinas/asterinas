// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

#define ROOT "/ext-space"
#define BLOCK_SIZE 4096

FN_SETUP(check_disposable_image)
{
	struct statfs st;
	CHECK_WITH(statfs(ROOT, &st), _ret == 0 && st.f_bsize == BLOCK_SIZE &&
					      st.f_blocks == 2035);
}
END_SETUP()

/* Only the dedicated, 8 MiB EXT2 regression image may be filled. */
FN_TEST(fallocate_enospc_preserves_size_and_free_counts)
{
	struct statfs before, full, after;
	TEST_SUCC(statfs(ROOT, &before));
	int fd = TEST_SUCC(
		open(ROOT "/target", O_CREAT | O_EXCL | O_RDWR, 0600));
	int filler = TEST_SUCC(
		open(ROOT "/filler", O_CREAT | O_EXCL | O_RDWR, 0600));
	TEST_SUCC(fallocate(fd, 0, 0, 2 * BLOCK_SIZE));
	off_t offset = 0;
	int result = 0, allocation_error = 0;
	/* One-block requests keep failure deterministic without a huge buffer. */
	while (offset < 2048 * BLOCK_SIZE) {
		errno = 0;
		result = fallocate(filler, 0, offset, BLOCK_SIZE);
		allocation_error = errno;
		if (result != 0)
			break;
		offset += BLOCK_SIZE;
	}
	TEST_RES(result, _ret == -1 && allocation_error == ENOSPC);
	TEST_SUCC(statfs(ROOT, &full));
	/* FIXME: Asterinas's indirect allocator can leave a usable direct block
	 * after a failed indirect-node allocation. Linux EXT4 may report free
	 * blocks unavailable to this request because of its allocation reserves.
	 * Keep the exact exhaustion check specific to the Asterinas allocator
	 * until both allocators expose the same remaining-space semantics.
	 */
	off_t size = 2 * BLOCK_SIZE;
#ifdef __asterinas__
	while (full.f_bfree && size < 12 * BLOCK_SIZE) {
		TEST_SUCC(fallocate(fd, 0, size, BLOCK_SIZE));
		size += BLOCK_SIZE;
		TEST_SUCC(statfs(ROOT, &full));
	}
	TEST_RES(full.f_bfree, _ret == 0);
#endif
	struct stat st;
	TEST_RES(fstat(fd, &st), st.st_size == size);
	TEST_ERRNO(fallocate(fd, 0, size, BLOCK_SIZE), ENOSPC);
	TEST_RES(fstat(fd, &st), st.st_size == size);
	TEST_RES(statfs(ROOT, &after), after.f_bfree == full.f_bfree &&
					       after.f_ffree == full.f_ffree);
	TEST_SUCC(close(filler));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(ROOT "/filler"));
	TEST_SUCC(unlink(ROOT "/target"));
	TEST_RES(statfs(ROOT, &after), after.f_bfree == before.f_bfree &&
					       after.f_ffree == before.f_ffree);
}
END_TEST()
