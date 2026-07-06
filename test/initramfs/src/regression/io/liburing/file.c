// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <liburing.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef SYS_io_uring_setup
#define SYS_io_uring_setup __NR_io_uring_setup
#endif

#ifndef SYS_io_uring_enter
#define SYS_io_uring_enter __NR_io_uring_enter
#endif

#define QUEUE_DEPTH 8
#define TEST_FILE "/tmp/io_uring_file_test"

static const char POSITIONAL_MESSAGE[] =
	"Asterinas io_uring positional file I/O";
static const char CURRENT_OFFSET_MESSAGE[] =
	"Asterinas io_uring current-offset file I/O";

enum {
	FILE_POSITIONAL_WRITE_USER_DATA = 0x102,
	FILE_POSITIONAL_READ_USER_DATA = 0x103,
	FILE_CURRENT_WRITE_USER_DATA = 0x104,
	FILE_CURRENT_READ_USER_DATA = 0x105,
	FILE_AFTER_DROP_USER_DATA = 0x106,
};

static void fail_errno(const char *message)
{
	perror(message);
	exit(EXIT_FAILURE);
}

static void fail_uring(const char *message, int ret)
{
	errno = -ret;
	perror(message);
	exit(EXIT_FAILURE);
}

static void fail_message(const char *message)
{
	fprintf(stderr, "%s\n", message);
	exit(EXIT_FAILURE);
}

static void init_ring(struct io_uring *ring)
{
	int ret = io_uring_queue_init(QUEUE_DEPTH, ring, 0);
	if (ret < 0) {
		fail_uring("io_uring_queue_init", ret);
	}
}

static struct io_uring_sqe *get_sqe(struct io_uring *ring,
				    const char *operation)
{
	struct io_uring_sqe *sqe = io_uring_get_sqe(ring);
	if (sqe == NULL) {
		fprintf(stderr, "%s: failed to get an SQE\n", operation);
		exit(EXIT_FAILURE);
	}

	return sqe;
}

static void submit_one(struct io_uring *ring, const char *operation)
{
	int ret = io_uring_submit(ring);
	if (ret < 0) {
		fail_uring(operation, ret);
	}
	if (ret != 1) {
		fprintf(stderr, "%s: submitted %d SQEs, expected 1\n",
			operation, ret);
		exit(EXIT_FAILURE);
	}
}

static int wait_cqe_res_user_data(struct io_uring *ring, const char *operation,
				  unsigned long long expected_user_data)
{
	struct io_uring_cqe *cqe;
	int ret = io_uring_wait_cqe(ring, &cqe);
	if (ret < 0) {
		fail_uring(operation, ret);
	}

	if ((unsigned long long)cqe->user_data != expected_user_data) {
		fprintf(stderr, "%s: got user_data %llu, expected %llu\n",
			operation, (unsigned long long)cqe->user_data,
			expected_user_data);
		exit(EXIT_FAILURE);
	}

	int res = cqe->res;
	io_uring_cqe_seen(ring, cqe);
	if (res < 0) {
		fail_uring(operation, res);
	}

	return res;
}

static void expect_cqe_res_user_data(struct io_uring *ring,
				     const char *operation,
				     unsigned long long expected_user_data,
				     int expected_res)
{
	int res = wait_cqe_res_user_data(ring, operation, expected_user_data);
	if (res != expected_res) {
		fprintf(stderr, "%s: got CQE result %d, expected %d\n",
			operation, res, expected_res);
		exit(EXIT_FAILURE);
	}
}

static void test_invalid_sq_array_dropped(struct io_uring *ring)
{
	unsigned dropped_before = *ring->sq.kdropped;
	struct io_uring_sqe *sqe = get_sqe(ring, "io_uring invalid SQ array");
	io_uring_prep_nop(sqe);

	unsigned slot = (ring->sq.sqe_tail - 1) & *ring->sq.kring_mask;
	ring->sq.array[slot] = *ring->sq.kring_entries;

	int ret = io_uring_submit(ring);
	ring->sq.array[slot] = slot;
	if (ret < 0) {
		fail_uring("io_uring invalid SQ array submit", ret);
	}
	if (ret != 0) {
		fprintf(stderr,
			"io_uring invalid SQ array: submitted %d SQEs, expected 0\n",
			ret);
		exit(EXIT_FAILURE);
	}
	if (*ring->sq.kdropped != dropped_before + 1) {
		fprintf(stderr,
			"io_uring invalid SQ array: dropped %u, expected %u\n",
			*ring->sq.kdropped, dropped_before + 1);
		exit(EXIT_FAILURE);
	}
	if (*ring->cq.ktail - *ring->cq.khead != 0) {
		fail_message(
			"io_uring invalid SQ array unexpectedly posted a CQE");
	}

	sqe = get_sqe(ring, "io_uring NOP after invalid SQ array");
	io_uring_prep_nop(sqe);
	sqe->user_data = FILE_AFTER_DROP_USER_DATA;
	submit_one(ring, "io_uring NOP after invalid SQ array submit");
	expect_cqe_res_user_data(ring, "io_uring NOP after invalid SQ array",
				 FILE_AFTER_DROP_USER_DATA, 0);
	printf("[io_uring:file] invalid SQ array dropped\n");
}

static void test_positional_read_write(struct io_uring *ring)
{
	int fd = open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR, 0600);
	if (fd < 0) {
		fail_errno("open");
	}

	struct io_uring_sqe *sqe = get_sqe(ring, "io_uring file write");
	io_uring_prep_write(sqe, fd, POSITIONAL_MESSAGE,
			    sizeof(POSITIONAL_MESSAGE), 0);
	sqe->user_data = FILE_POSITIONAL_WRITE_USER_DATA;
	submit_one(ring, "io_uring file write submit");
	expect_cqe_res_user_data(ring, "io_uring file write",
				 FILE_POSITIONAL_WRITE_USER_DATA,
				 sizeof(POSITIONAL_MESSAGE));
	printf("[io_uring:file] write completed\n");

	char read_buffer[sizeof(POSITIONAL_MESSAGE)] = {};
	sqe = get_sqe(ring, "io_uring file read");
	io_uring_prep_read(sqe, fd, read_buffer, sizeof(read_buffer), 0);
	sqe->user_data = FILE_POSITIONAL_READ_USER_DATA;
	submit_one(ring, "io_uring file read submit");
	expect_cqe_res_user_data(ring, "io_uring file read",
				 FILE_POSITIONAL_READ_USER_DATA,
				 sizeof(read_buffer));

	if (memcmp(read_buffer, POSITIONAL_MESSAGE,
		   sizeof(POSITIONAL_MESSAGE)) != 0) {
		fail_message("io_uring file read returned unexpected data");
	}
	printf("[io_uring:file] positional read data verified\n");

	if (close(fd) < 0) {
		fail_errno("close");
	}
	if (unlink(TEST_FILE) < 0) {
		fail_errno("unlink");
	}
}

static void test_current_offset_read_write(struct io_uring *ring)
{
	int fd = open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR, 0600);
	if (fd < 0) {
		fail_errno("open");
	}

	struct io_uring_sqe *sqe =
		get_sqe(ring, "io_uring current-offset write");
	io_uring_prep_write(sqe, fd, CURRENT_OFFSET_MESSAGE,
			    sizeof(CURRENT_OFFSET_MESSAGE), -1);
	sqe->user_data = FILE_CURRENT_WRITE_USER_DATA;
	submit_one(ring, "io_uring current-offset write submit");
	expect_cqe_res_user_data(ring, "io_uring current-offset write",
				 FILE_CURRENT_WRITE_USER_DATA,
				 sizeof(CURRENT_OFFSET_MESSAGE));

	if (lseek(fd, 0, SEEK_SET) < 0) {
		fail_errno("lseek");
	}

	char read_buffer[sizeof(CURRENT_OFFSET_MESSAGE)] = {};
	sqe = get_sqe(ring, "io_uring current-offset read");
	io_uring_prep_read(sqe, fd, read_buffer, sizeof(read_buffer), -1);
	sqe->user_data = FILE_CURRENT_READ_USER_DATA;
	submit_one(ring, "io_uring current-offset read submit");
	expect_cqe_res_user_data(ring, "io_uring current-offset read",
				 FILE_CURRENT_READ_USER_DATA,
				 sizeof(read_buffer));

	if (memcmp(read_buffer, CURRENT_OFFSET_MESSAGE,
		   sizeof(CURRENT_OFFSET_MESSAGE)) != 0) {
		fail_message(
			"io_uring current-offset read returned unexpected data");
	}
	printf("[io_uring:file] current-offset read data verified\n");

	if (close(fd) < 0) {
		fail_errno("close");
	}
	if (unlink(TEST_FILE) < 0) {
		fail_errno("unlink");
	}
}

int main(void)
{
	setbuf(stdout, NULL);
	alarm(30);
	printf("[io_uring:file] starting\n");

	struct io_uring ring;
	init_ring(&ring);
	test_invalid_sq_array_dropped(&ring);
	test_positional_read_write(&ring);
	test_current_offset_read_write(&ring);
	io_uring_queue_exit(&ring);

	printf("[io_uring:file] PASS\n");
	return EXIT_SUCCESS;
}
