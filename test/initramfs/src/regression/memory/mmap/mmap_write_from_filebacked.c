// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define SRC_FILE "/ext2/mmap_write_from_filebacked.src"
#define DST_FILE_PREFIX "/tmp/mmap_write_from_filebacked.dst."

#define PAGE_SIZE 4096
#define TEST_PAGES 64
#define TEST_SIZE (PAGE_SIZE * TEST_PAGES)
#define NR_THREADS 48
#define NR_HOLDERS 48
#define RACE_PAGE 1

static char *src_map;
static int src_fd;
static int dst_fds[NR_THREADS];
static pthread_t threads[NR_THREADS];
static pthread_t page_cache_holders[NR_HOLDERS];
static atomic_int ready_count;
static atomic_int holder_ready_count;
static atomic_int holder_start;
static atomic_int start;
static volatile unsigned char sink;

static void write_all(int fd, const void *buf, size_t len)
{
	const char *cursor = buf;
	size_t remaining = len;

	while (remaining > 0) {
		ssize_t written = CHECK(write(fd, cursor, remaining));

		if (written == 0) {
			fprintf(stderr, "fatal error: write made no progress\n");
			exit(EXIT_FAILURE);
		}

		cursor += written;
		remaining -= written;
	}
}

static void create_source_file(void)
{
	void *buf = NULL;

	CHECK(posix_memalign(&buf, PAGE_SIZE, TEST_SIZE));
	memset(buf, 0x5a, TEST_SIZE);

	src_fd = CHECK(open(SRC_FILE, O_RDWR | O_CREAT | O_TRUNC | O_DIRECT,
			    0666));
	CHECK(unlink(SRC_FILE));
	write_all(src_fd, buf, TEST_SIZE);

	/*
	 * Direct I/O updates the inode size but deliberately bypasses the page
	 * cache. Toggle the file size once so the VMO size is correct while its
	 * pages remain uncommitted.
	 */
	CHECK(ftruncate(src_fd, TEST_SIZE + PAGE_SIZE));
	CHECK(ftruncate(src_fd, TEST_SIZE));
	free(buf);
}

static void *write_from_file_mapping(void *arg)
{
	int thread_id = (intptr_t)arg;

	atomic_fetch_add_explicit(&ready_count, 1, memory_order_release);
	while (!atomic_load_explicit(&start, memory_order_acquire)) {
		sched_yield();
	}

	write_all(dst_fds[thread_id], src_map, TEST_SIZE);
	return NULL;
}

static void *fault_file_mapping_normally(void *arg)
{
	(void)arg;

	atomic_fetch_add_explicit(&holder_ready_count, 1, memory_order_release);
	while (!atomic_load_explicit(&holder_start, memory_order_acquire)) {
		sched_yield();
	}

	sink ^= src_map[RACE_PAGE * PAGE_SIZE];
	return NULL;
}

FN_SETUP(mmap_write_from_filebacked)
{
	int i;

	create_source_file();
	src_map = CHECK_WITH(mmap(NULL, TEST_SIZE, PROT_READ, MAP_SHARED,
				  src_fd, 0),
			     _ret != MAP_FAILED);

	for (i = 0; i < NR_THREADS; i++) {
		char path[sizeof(DST_FILE_PREFIX) + 12];

		CHECK_WITH(snprintf(path, sizeof(path), "%s%d", DST_FILE_PREFIX,
				    i),
			   _ret > 0 && (size_t)_ret < sizeof(path));
		dst_fds[i] = CHECK(open(path, O_RDWR | O_CREAT | O_TRUNC, 0666));
		CHECK(unlink(path));
	}
}
END_SETUP()

FN_TEST(mmap_write_from_filebacked)
{
	int i;

	sink ^= src_map[0];

	for (i = 0; i < NR_THREADS; i++) {
		TEST_RES(pthread_create(&threads[i], NULL,
					write_from_file_mapping,
					(void *)(intptr_t)i),
			 _ret == 0);
	}
	for (i = 0; i < NR_HOLDERS; i++) {
		TEST_RES(pthread_create(&page_cache_holders[i], NULL,
					fault_file_mapping_normally, NULL),
			 _ret == 0);
	}

	while (atomic_load_explicit(&ready_count, memory_order_acquire) <
	       NR_THREADS) {
		sched_yield();
	}
	while (atomic_load_explicit(&holder_ready_count, memory_order_acquire) <
	       NR_HOLDERS) {
		sched_yield();
	}
	atomic_store_explicit(&holder_start, 1, memory_order_release);
	for (i = 0; i < 1000; i++) {
		sched_yield();
	}
	atomic_store_explicit(&start, 1, memory_order_release);

	for (i = 0; i < NR_THREADS; i++) {
		TEST_RES(pthread_join(threads[i], NULL), _ret == 0);
	}
	for (i = 0; i < NR_HOLDERS; i++) {
		TEST_RES(pthread_join(page_cache_holders[i], NULL), _ret == 0);
	}
}
END_TEST()
