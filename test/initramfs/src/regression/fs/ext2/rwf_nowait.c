// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/rwf_nowait"
#ifndef RWF_NOWAIT
#define RWF_NOWAIT 0x8
#endif
#define PAGE_SIZE 4096

// File-scope buffers keep the O_DIRECT buffers block-aligned.
static char buf[PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));
static char buf2[2 * PAGE_SIZE] __attribute__((aligned(PAGE_SIZE)));

FN_SETUP(prepare_base_dir)
{
	CHECK_WITH(mkdir(BASE_DIR, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

FN_TEST(read_cold_page_nowait_eagain)
{
	const char *path = BASE_DIR "/cold_page";
	struct iovec iov = { .iov_base = buf, .iov_len = PAGE_SIZE };
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_SUCC(ftruncate(fd, 2 * PAGE_SIZE));
	TEST_ERRNO(syscall(SYS_preadv2, fd, &iov, 1, PAGE_SIZE, 0, RWF_NOWAIT),
		   EAGAIN);

	TEST_RES(pread(fd, buf, PAGE_SIZE, PAGE_SIZE), _ret == PAGE_SIZE);
	TEST_RES(syscall(SYS_preadv2, fd, &iov, 1, PAGE_SIZE, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(write_buffered_nowait_eopnotsupp)
{
	const char *path = BASE_DIR "/buffered_write";
	struct iovec iov = { .iov_base = buf, .iov_len = 1 };
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_ERRNO(syscall(SYS_pwritev2, fd, &iov, 1, 0, 0, RWF_NOWAIT),
		   EOPNOTSUPP);

	TEST_RES(write(fd, buf, 1), _ret == 1);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(write_dio_hole_nowait_eagain)
{
	const char *path = BASE_DIR "/dio_write";
	struct iovec iov = { .iov_base = buf, .iov_len = PAGE_SIZE };
	int fd = TEST_SUCC(
		open(path, O_RDWR | O_CREAT | O_TRUNC | O_DIRECT, 0600));

	// The first write extends the file, so this EAGAIN comes from the
	// extending check rather than the hole check.
	TEST_ERRNO(syscall(SYS_pwritev2, fd, &iov, 1, 0, 0, RWF_NOWAIT),
		   EAGAIN);

	TEST_RES(syscall(SYS_pwritev2, fd, &iov, 1, 0, 0, 0),
		 _ret == PAGE_SIZE);
	TEST_RES(syscall(SYS_pwritev2, fd, &iov, 1, 0, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_SUCC(ftruncate(fd, 2 * PAGE_SIZE));
	TEST_ERRNO(syscall(SYS_pwritev2, fd, &iov, 1, PAGE_SIZE, 0, RWF_NOWAIT),
		   EAGAIN);

	TEST_RES(syscall(SYS_pwritev2, fd, &iov, 1, PAGE_SIZE, 0, 0),
		 _ret == PAGE_SIZE);
	TEST_RES(syscall(SYS_pwritev2, fd, &iov, 1, PAGE_SIZE, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(read_dio_dirty_cache_nowait_eagain)
{
	const char *path = BASE_DIR "/dio_read";
	struct iovec iov = { .iov_base = buf, .iov_len = PAGE_SIZE };
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));
	int dio_fd = TEST_SUCC(open(path, O_RDONLY | O_DIRECT));

	TEST_RES(write(fd, buf, PAGE_SIZE), _ret == PAGE_SIZE);
	TEST_ERRNO(syscall(SYS_preadv2, dio_fd, &iov, 1, 0, 0, RWF_NOWAIT),
		   EAGAIN);

	TEST_SUCC(fsync(fd));
	TEST_RES(syscall(SYS_preadv2, dio_fd, &iov, 1, 0, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_SUCC(close(dio_fd));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(read_partial_cache_nowait_short_read)
{
	const char *path = BASE_DIR "/partial_cache";
	struct iovec iov = { .iov_base = buf2, .iov_len = 2 * PAGE_SIZE };
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_SUCC(ftruncate(fd, 2 * PAGE_SIZE));
	TEST_RES(pread(fd, buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);

	// RWF_NOWAIT returns a short read instead of EAGAIN when only part
	// of the range is cached.
	TEST_RES(syscall(SYS_preadv2, fd, &iov, 1, 0, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_RES(pread(fd, buf, PAGE_SIZE, PAGE_SIZE), _ret == PAGE_SIZE);
	TEST_RES(syscall(SYS_preadv2, fd, &iov, 1, 0, 0, RWF_NOWAIT),
		 _ret == 2 * PAGE_SIZE);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(write_dio_cached_pages_nowait_eagain)
{
	const char *path = BASE_DIR "/dio_write_cache";
	struct iovec iov = { .iov_base = buf, .iov_len = PAGE_SIZE };
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));
	int dio_fd = TEST_SUCC(open(path, O_RDWR | O_DIRECT));

	TEST_RES(write(fd, buf, PAGE_SIZE), _ret == PAGE_SIZE);

	// Any cached page in the range reports EAGAIN; a blocking direct
	// write clears the cache for the next NOWAIT attempt.
	TEST_ERRNO(syscall(SYS_pwritev2, dio_fd, &iov, 1, 0, 0, RWF_NOWAIT),
		   EAGAIN);

	TEST_RES(syscall(SYS_pwritev2, dio_fd, &iov, 1, 0, 0, 0),
		 _ret == PAGE_SIZE);
	TEST_RES(syscall(SYS_pwritev2, dio_fd, &iov, 1, 0, 0, RWF_NOWAIT),
		 _ret == PAGE_SIZE);

	TEST_SUCC(close(dio_fd));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()
