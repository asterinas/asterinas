// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/falloc.h>
#include <signal.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define BIG_SIZE ((off_t)1 << 30)
#define TIME_LIMIT_MS 5000

static double elapsed_ms(struct timespec *begin, struct timespec *end)
{
	return (double)(end->tv_sec - begin->tv_sec) * 1000.0 +
	       (double)(end->tv_nsec - begin->tv_nsec) / 1000000.0;
}

FN_SETUP(create_file)
{
	int fd = CHECK(
		open("/tmp/ramfs_fallocate", O_RDWR | O_CREAT | O_TRUNC, 0644));
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(write_checks_offset_and_size_limits)
{
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));

	// Invalid signed offset ranges fail without extending the file.
	TEST_RES(lseek(fd, INT64_MAX, SEEK_SET), _ret == INT64_MAX);
	TEST_RES(write(fd, "", 0), _ret == 0);
	TEST_ERRNO(write(fd, "x", 1), EINVAL);
	TEST_ERRNO(pwrite(fd, "x", 1, INT64_MAX), EINVAL);
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == INT64_MAX);

	struct stat st;
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);

	// The requested offset is checked before O_APPEND selects EOF.
	int append_fd =
		TEST_SUCC(open("/tmp/ramfs_fallocate", O_WRONLY | O_APPEND));
	TEST_RES(lseek(append_fd, INT64_MAX, SEEK_SET), _ret == INT64_MAX);
	TEST_ERRNO(write(append_fd, "x", 1), EINVAL);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);
	TEST_RES(lseek(append_fd, 0, SEEK_SET), _ret == 0);

	// A valid requested offset can still exceed the filesystem's size limit.
	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));
	struct rlimit limit = { RLIM_INFINITY, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));
	TEST_SUCC(ftruncate(fd, INT64_MAX));
	TEST_ERRNO(write(append_fd, "x", 1), EFBIG);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 0);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	TEST_SUCC(ftruncate(fd, 0));
	TEST_RES(write(append_fd, "x", 1), _ret == 1);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 1);

	// Crossing the size limit writes the remaining byte.
	TEST_SUCC(ftruncate(fd, INT64_MAX - 1));
	TEST_RES(lseek(append_fd, 0, SEEK_SET), _ret == 0);
	TEST_RES(write(append_fd, "xy", 2), _ret == 1);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == INT64_MAX);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	char byte;
	TEST_RES(pread(fd, &byte, 1, INT64_MAX - 1), _ret == 1 && byte == 'x');

	// An empty append leaves the descriptor offset and file size unchanged.
	TEST_RES(lseek(append_fd, 7, SEEK_SET), _ret == 7);
	TEST_RES(write(append_fd, "", 0), _ret == 0);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 7);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	TEST_SUCC(ftruncate(fd, 0));
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(close(append_fd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(allocate_checks_signed_range)
{
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR | O_TRUNC));
	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));
	struct rlimit limit = { RLIM_INFINITY, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));

	static const int modes[] = {
		0,
		FALLOC_FL_KEEP_SIZE,
		FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
	};
	for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
		TEST_ERRNO(fallocate(fd, modes[i], INT64_MAX, 1), EFBIG);
	}
	TEST_ERRNO(fallocate(-1, 0, INT64_MAX, 1), EBADF);
	struct stat st;
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(close(fd));
}
END_TEST()

// `fallocate` refuses a range that exceeds `RLIMIT_FSIZE`.
FN_TEST(allocate_respects_rlimit_fsize)
{
	// Ignore SIGXFSZ so Linux returns EFBIG to the test.
	struct sigaction action = { .sa_handler = SIG_IGN };
	struct sigaction old_action;
	TEST_SUCC(sigemptyset(&action.sa_mask));
	TEST_SUCC(sigaction(SIGXFSZ, &action, &old_action));

	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));

	struct rlimit limit = { 4096, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));

	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));
	TEST_ERRNO(fallocate(fd, 0, 0, 8192), EFBIG);
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
	TEST_SUCC(close(fd));

	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(sigaction(SIGXFSZ, &old_action, NULL));
}
END_TEST()

// Regression for #4081: a sparse hole must not allocate a page at every offset.
FN_TEST(punch_hole_is_cheap)
{
	// Allow the large file regardless of the inherited soft limit.
	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));
	struct rlimit limit = { (rlim_t)BIG_SIZE, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));

	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));

	TEST_SUCC(ftruncate(fd, BIG_SIZE));

	struct timespec begin;
	struct timespec end;
	TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &begin));
	TEST_SUCC(fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 0,
			    BIG_SIZE));
	TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &end));

	TEST_RES(elapsed_ms(&begin, &end), _ret <= TIME_LIMIT_MS);

	struct stat st;
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == BIG_SIZE);

	TEST_SUCC(close(fd));
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
}
END_TEST()

// A hole punched in the middle of a file keeps the data before and after it.
FN_TEST(punch_hole_preserves_data)
{
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));

	char data[8192];
	for (size_t i = 0; i < sizeof(data); i++) {
		data[i] = (char)('a' + i % 26);
	}
	TEST_RES(pwrite(fd, data, sizeof(data), 0), _ret == sizeof(data));

	TEST_SUCC(fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
			    4096, 4096));

	char buf[4096];
	TEST_RES(pread(fd, buf, sizeof(buf), 0), _ret == sizeof(buf));
	TEST_RES(memcmp(buf, data, sizeof(buf)), _ret == 0);

	TEST_RES(pread(fd, buf, sizeof(buf), 4096), _ret == sizeof(buf));
	int all_zero = 1;
	for (size_t i = 0; i < sizeof(buf); i++) {
		if (buf[i] != 0) {
			all_zero = 0;
		}
	}
	TEST_RES(all_zero, _ret == 1);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(punch_hole_boundaries)
{
	static const struct {
		off_t offset;
		off_t len;
	} ranges[] = {
		// Partial ranges within one page.
		{ 100, 100 },
		{ 0, 100 },
		{ 100, 3996 },
		{ 4096, 100 },

		// A complete page and a range crossing a page boundary.
		{ 4096, 4096 },
		{ 4000, 200 },

		// Partial pages around a complete page (xfstests generic/567).
		{ 2048, 8192 },

		// Clip at EOF, or leave the file unchanged when starting at EOF.
		{ 10000, 8192 },
		{ 12288, 4096 },
	};
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR | O_TRUNC));
	char expected[12288];
	char actual[sizeof(expected)];

	for (size_t i = 0; i < sizeof(ranges) / sizeof(ranges[0]); i++) {
		memset(expected, 'X', sizeof(expected));
		TEST_RES(pwrite(fd, expected, sizeof(expected), 0),
			 _ret == sizeof(expected));
		char *mapping = TEST_RES(mmap(NULL, sizeof(expected),
					      PROT_READ | PROT_WRITE,
					      MAP_SHARED, fd, 0),
					 _ret != MAP_FAILED);
		// Populate writable mappings before punching the hole.
		memset(mapping, 'X', sizeof(expected));
		TEST_SUCC(fallocate(fd,
				    FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
				    ranges[i].offset, ranges[i].len));
		size_t start = ranges[i].offset;
		size_t end = start + ranges[i].len;
		if (end > sizeof(expected)) {
			end = sizeof(expected);
		}
		memset(expected + start, 0, end - start);
		TEST_RES(pread(fd, actual, sizeof(actual), 0),
			 _ret == sizeof(actual));
		TEST_RES(memcmp(actual, expected, sizeof(actual)), _ret == 0);
		TEST_RES(memcmp(mapping, expected, sizeof(expected)),
			 _ret == 0);
		memset(mapping + start, 'Y', end - start);
		memset(expected + start, 'Y', end - start);
		TEST_RES(pread(fd, actual, sizeof(actual), 0),
			 _ret == sizeof(actual));
		TEST_RES(memcmp(actual, expected, sizeof(actual)), _ret == 0);
		struct stat st;
		TEST_RES(fstat(fd, &st),
			 _ret == 0 && st.st_size == sizeof(expected));
		TEST_SUCC(munmap(mapping, sizeof(expected)));
	}
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(remove_file)
{
	CHECK(unlink("/tmp/ramfs_fallocate"));
}
END_SETUP()
