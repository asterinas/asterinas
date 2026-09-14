/* SPDX-License-Identifier: MPL-2.0 */

#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <unistd.h>

#include "../../common/test.h"

#define FILE_NAME "aster_read_eof"
#define CONTENT "hi"
#define CONTENT_LEN (sizeof(CONTENT) - 1)
#define BUF_SIZE 64
#define SENTINEL 0xAA
#define READV_BUF_SIZE 8192
#define READV_ROUNDS 20000
/* Use extra writers to increase the chance of cross-CPU placement. */
#define NR_WRITERS 8

/* Mount points of the file systems whose reads go through the page cache. */
static const char *const DIRS[] = { "/tmp", "/ext2", "/exfat" };
#define NR_DIRS (sizeof(DIRS) / sizeof(DIRS[0]))

static int fds[NR_DIRS];

/* Returns 1 if `buf[from..BUF_SIZE)` still holds the sentinel. */
static int tail_untouched(const unsigned char *buf, size_t from)
{
	for (size_t i = from; i < BUF_SIZE; i++) {
		if (buf[i] != SENTINEL) {
			return 0;
		}
	}

	return 1;
}

FN_SETUP(create_files)
{
	char path[64];
	struct stat st;

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (stat(DIRS[i], &st) < 0) {
			fds[i] = -1;
			continue;
		}

		CHECK(snprintf(path, sizeof(path), "%s/" FILE_NAME, DIRS[i]));
		unlink(path);

		fds[i] = CHECK(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));
		CHECK_WITH(write(fds[i], CONTENT, CONTENT_LEN),
			   _ret == CONTENT_LEN);
	}
}
END_SETUP()

FN_TEST(pread_across_eof_leaves_tail_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_RES(pread(fds[i], buf, sizeof(buf), 1),
			 _ret == CONTENT_LEN - 1);
		TEST_RES(tail_untouched(buf, CONTENT_LEN - 1), _ret == 1);
	}
}
END_TEST()

FN_TEST(pread_at_eof_leaves_buffer_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_RES(pread(fds[i], buf, sizeof(buf), CONTENT_LEN),
			 _ret == 0);
		TEST_RES(tail_untouched(buf, 0), _ret == 1);
	}
}
END_TEST()

FN_TEST(read_across_eof_leaves_tail_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_SUCC(lseek(fds[i], 1, SEEK_SET));
		TEST_RES(read(fds[i], buf, sizeof(buf)),
			 _ret == CONTENT_LEN - 1);
		TEST_RES(tail_untouched(buf, CONTENT_LEN - 1), _ret == 1);
	}
}
END_TEST()

static _Atomic int stop_requested;
static _Atomic int writer_cpus[NR_WRITERS];

static int current_cpu(void)
{
	unsigned int cpu;

	CHECK(syscall(SYS_getcpu, &cpu, NULL, NULL));
	return cpu;
}

/* Keep the file smaller than iov[0], so an append after a short read
 * must not place data in iov[1].
 */
static void *append_and_truncate(void *arg)
{
	int fd = fds[0];

	*(_Atomic int *)arg = current_cpu();
	while (!stop_requested) {
		CHECK_WITH(pwrite(fd, "!", 1, CONTENT_LEN), _ret == 1);
		CHECK(ftruncate(fd, CONTENT_LEN));
	}

	return NULL;
}

static void start_writers(pthread_t *threads)
{
	for (int i = 0; i < NR_WRITERS; i++) {
		writer_cpus[i] = -1;
		CHECK_WITH(pthread_create(&threads[i], NULL,
					  append_and_truncate, &writer_cpus[i]),
			   _ret == 0);
	}
	for (int i = 0; i < NR_WRITERS; i++) {
		while (writer_cpus[i] < 0)
			sched_yield();
	}
}

/* Must run before the test returns, cleanup closes the fd under the writers. */
static void stop_writers(pthread_t *threads)
{
	stop_requested = 1;
	for (int i = 0; i < NR_WRITERS; i++) {
		CHECK_WITH(pthread_join(threads[i], NULL), _ret == 0);
	}
}

static long count_split_reads(int fd)
{
	/* Larger than the one cached page, so the pre-fix whole-page copy
	 * leaves space in iov[0] and stops before iov[1].
	 */
	static unsigned char first[READV_BUF_SIZE];
	static unsigned char second[1];
	struct iovec iov[2] = {
		{ .iov_base = first, .iov_len = sizeof(first) },
		{ .iov_base = second, .iov_len = sizeof(second) },
	};
	long split = 0;

	for (long i = 0; i < READV_ROUNDS; i++) {
		memset(first, SENTINEL, sizeof(first));
		second[0] = SENTINEL;
		ssize_t ret = CHECK(preadv(fd, iov, 2, 0));
		int bad = second[0] != SENTINEL || ret > CONTENT_LEN + 1;
		for (ssize_t j = 0; j < ret && j < READV_BUF_SIZE; j++) {
			if (first[j] == SENTINEL)
				bad = 1;
		}
		if (bad)
			split++;
	}

	return split;
}

FN_TEST(preadv_fills_iovecs_in_order)
{
	/* A short first iovec must stop the read even if an append follows. */
	int fd = fds[0];
	SKIP_TEST_IF(fd < 0);
	SKIP_TEST_IF(sysconf(_SC_NPROCESSORS_ONLN) < 2);

	pthread_t threads[NR_WRITERS];
	start_writers(threads);

	/* This placement check assumes threads do not migrate between CPUs. */
	int reader_cpu = current_cpu();
	int all_on_reader_cpu = 1;
	for (int i = 0; i < NR_WRITERS; i++) {
		if (writer_cpus[i] != reader_cpu)
			all_on_reader_cpu = 0;
	}
	if (all_on_reader_cpu)
		stop_writers(threads);
	SKIP_TEST_IF(all_on_reader_cpu);

	TEST_RES(count_split_reads(fd), _ret == 0);
	stop_writers(threads);
}
END_TEST()

FN_SETUP(cleanup)
{
	char path[64];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}

		CHECK(close(fds[i]));
		CHECK(snprintf(path, sizeof(path), "%s/" FILE_NAME, DIRS[i]));
		CHECK(unlink(path));
	}
}
END_SETUP()
