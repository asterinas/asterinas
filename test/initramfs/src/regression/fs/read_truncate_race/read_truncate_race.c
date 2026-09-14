/* SPDX-License-Identifier: MPL-2.0 */

#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

/*
 * A read that races with a truncate or an extending write may be short, but it
 * must never report bytes it did not copy. The reader fills its buffer with a
 * sentinel before every `pread()`; the file only ever holds 'x', so a sentinel
 * byte inside `[0, ret)` is a byte the kernel counted but never wrote.
 */

#define FILE_PATH "/tmp/aster_read_truncate_race"
#define LEN 8192
#define ROUNDS 20000
#define SENTINEL 0xAA
/* Use extra writers to increase the chance of cross-CPU placement. */
#define NR_WRITERS 8

static int fd;
static char data[LEN];
static _Atomic int stop_requested;
static _Atomic int writer_cpus[NR_WRITERS];

static int current_cpu(void)
{
	unsigned int cpu;

	CHECK(syscall(SYS_getcpu, &cpu, NULL, NULL));
	return cpu;
}

/* Alternates the file between empty and LEN bytes of 'x'. */
static void *truncate_and_rewrite(void *arg)
{
	*(_Atomic int *)arg = current_cpu();
	while (!stop_requested) {
		CHECK(ftruncate(fd, 0));
		CHECK_WITH(pwrite(fd, data, LEN, 0), _ret == LEN);
	}

	return NULL;
}

static void start_writers(pthread_t *threads)
{
	for (int i = 0; i < NR_WRITERS; i++) {
		writer_cpus[i] = -1;
		CHECK_WITH(pthread_create(&threads[i], NULL,
					  truncate_and_rewrite,
					  &writer_cpus[i]),
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

/* Returns how many of ROUNDS reads reported bytes that were never written. */
static long count_unwritten_reads(void)
{
	static unsigned char buf[LEN];
	long unwritten = 0;

	for (long i = 0; i < ROUNDS; i++) {
		memset(buf, SENTINEL, LEN);
		ssize_t ret = CHECK(pread(fd, buf, LEN, 0));
		for (ssize_t j = 0; j < ret; j++) {
			if (buf[j] == SENTINEL) {
				unwritten++;
				break;
			}
		}
	}

	return unwritten;
}

FN_SETUP(create_file)
{
	memset(data, 'x', LEN);
	fd = CHECK(open(FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0600));
	CHECK_WITH(pwrite(fd, data, LEN, 0), _ret == LEN);
}
END_SETUP()

FN_TEST(read_never_reports_unwritten_bytes)
{
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

	TEST_RES(count_unwritten_reads(), _ret == 0);
	stop_writers(threads);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(FILE_PATH));
}
END_SETUP()
