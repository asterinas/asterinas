/* SPDX-License-Identifier: MPL-2.0 */

/*
 * A short read at EOF must stop before the next iovec, even if another thread
 * appends data before that iovec could be read.
 */

#define _GNU_SOURCE

#include <fcntl.h>
#include <pthread.h>
#include <sys/uio.h>
#include <unistd.h>

#include "../../common/test.h"

#define FILE_PATH "/ext2/aster_preadv_short_read"
#define BLOCK_SIZE 4096
#define SENTINEL 0xAA
#define ROUNDS 20000
/* With one writer, an append rarely lands between two `O_DIRECT` reads. */
#define NR_WRITERS 2

static int fd;
static int direct_fd;
static unsigned char data[2 * BLOCK_SIZE];
static _Atomic int stop_requested;

FN_SETUP(create_file)
{
	memset(data, '!', sizeof(data));
	fd = CHECK(open(FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0600));
	CHECK_WITH(write(fd, data, BLOCK_SIZE), _ret == BLOCK_SIZE);
	direct_fd = CHECK(open(FILE_PATH, O_RDONLY | O_DIRECT));
}
END_SETUP()

/* Keeps the file one or two blocks long. */
static void *append_and_truncate(void *arg)
{
	while (!stop_requested) {
		CHECK_WITH(pwrite(fd, data, BLOCK_SIZE, BLOCK_SIZE),
			   _ret == BLOCK_SIZE);
		CHECK(ftruncate(fd, BLOCK_SIZE));
	}

	return NULL;
}

/*
 * Each read must copy the returned bytes into the first iovec and leave the
 * rest of both buffers untouched. Count violations across all stress rounds.
 */
static long count_invalid_reads(int read_fd)
{
	/* `O_DIRECT` needs block-aligned buffers. */
	static unsigned char first[2 * BLOCK_SIZE]
		__attribute__((aligned(BLOCK_SIZE)));
	static unsigned char second[BLOCK_SIZE]
		__attribute__((aligned(BLOCK_SIZE)));

	unsigned char untouched[sizeof(first)];
	struct iovec iov[2] = {
		{ .iov_base = first, .iov_len = sizeof(first) },
		{ .iov_base = second, .iov_len = sizeof(second) },
	};
	long invalid = 0;

	memset(untouched, SENTINEL, sizeof(untouched));
	for (long i = 0; i < ROUNDS; i++) {
		memset(first, SENTINEL, sizeof(first));
		memset(second, SENTINEL, sizeof(second));
		ssize_t ret = preadv(read_fd, iov, 2, 0);
		if (ret < BLOCK_SIZE || ret > sizeof(first) ||
		    memcmp(first, data, ret) != 0 ||
		    memcmp(first + ret, untouched, sizeof(first) - ret) != 0 ||
		    memcmp(second, untouched, sizeof(second)) != 0)
			invalid++;
	}

	return invalid;
}

#define TEST_PREADV_ORDER(read_fd)                                            \
	do {                                                                  \
		pthread_t writers[NR_WRITERS];                                \
		stop_requested = 0;                                           \
		for (int i = 0; i < NR_WRITERS; i++)                          \
			TEST_SUCC(pthread_create(&writers[i], NULL,           \
						 append_and_truncate, NULL)); \
		TEST_RES(count_invalid_reads(read_fd), _ret == 0);            \
		stop_requested = 1;                                           \
		for (int i = 0; i < NR_WRITERS; i++)                          \
			TEST_SUCC(pthread_join(writers[i], NULL));            \
	} while (0)

FN_TEST(preadv_fills_iovecs_in_order)
{
	TEST_PREADV_ORDER(fd);
}
END_TEST()

FN_TEST(direct_preadv_fills_iovecs_in_order)
{
	TEST_PREADV_ORDER(direct_fd);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(direct_fd));
	CHECK(close(fd));
	CHECK(unlink(FILE_PATH));
}
END_SETUP()
