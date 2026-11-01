/* SPDX-License-Identifier: MPL-2.0 */

/*
 * A read racing with truncate must not report more bytes than it copied.
 */

#include <fcntl.h>
#include <pthread.h>
#include <unistd.h>

#include "../../common/test.h"

#define FILE_PATH "/tmp/aster_read_truncate_race"
#define LEN 8192
#define ROUNDS 20000
#define SENTINEL 0xAA

static int fd;
static char data[LEN];
static _Atomic int stop_requested;

FN_SETUP(create_file)
{
	memset(data, 'x', LEN);
	fd = CHECK(open(FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0600));
}
END_SETUP()

/* Alternates the file between empty and LEN bytes of 'x'. */
static void *truncate_and_rewrite(void *arg)
{
	while (!stop_requested) {
		CHECK(ftruncate(fd, 0));
		CHECK_WITH(pwrite(fd, data, LEN, 0), _ret == LEN);
	}

	return NULL;
}

/*
 * Every read must succeed and report only bytes copied into the buffer.
 * Count violations so the stress loop produces one assertion result.
 */
static long count_invalid_reads(void)
{
	static unsigned char buf[LEN];

	long invalid = 0;
	for (long i = 0; i < ROUNDS; i++) {
		memset(buf, SENTINEL, LEN);
		ssize_t ret = pread(fd, buf, LEN, 0);
		if (ret < 0 || ret > LEN || memchr(buf, SENTINEL, ret))
			invalid++;
	}

	return invalid;
}

FN_TEST(read_never_reports_unwritten_bytes)
{
	pthread_t writer;
	TEST_SUCC(pthread_create(&writer, NULL, truncate_and_rewrite, NULL));

	TEST_RES(count_invalid_reads(), _ret == 0);

	stop_requested = 1;
	TEST_SUCC(pthread_join(writer, NULL));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(FILE_PATH));
}
END_SETUP()
