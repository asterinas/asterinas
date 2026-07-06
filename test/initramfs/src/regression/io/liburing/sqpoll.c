// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <liburing.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define QUEUE_DEPTH 8
#define TEST_FILE "/tmp/io_uring_sqpoll_test"
#define SQPOLL_IDLE_TIMEOUT_MS 1
#define SQPOLL_SLEEP_RETRY_COUNT 1000

static const char SQPOLL_MESSAGE[] = "Asterinas io_uring SQPOLL file I/O";
static const char SQPOLL_WAKE_MESSAGE[] =
	"Asterinas io_uring SQPOLL wakeup file I/O";

enum {
	SQPOLL_WRITE_USER_DATA = 0x301,
	SQPOLL_READ_USER_DATA = 0x302,
	SQPOLL_WAKE_WRITE_USER_DATA = 0x303,
	SQPOLL_WAKE_READ_USER_DATA = 0x304,
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

static void init_sqpoll_ring(struct io_uring *ring)
{
	struct io_uring_params params = {
		.flags = IORING_SETUP_SQPOLL,
		.sq_thread_idle = SQPOLL_IDLE_TIMEOUT_MS,
	};

	int ret = io_uring_queue_init_params(QUEUE_DEPTH, ring, &params);
	if (ret < 0) {
		fail_uring("io_uring_queue_init_params SQPOLL", ret);
	}
}

static void wait_for_sqpoll_sleep(struct io_uring *ring)
{
	for (int i = 0; i < SQPOLL_SLEEP_RETRY_COUNT; i++) {
		if (*ring->sq.kflags & IORING_SQ_NEED_WAKEUP) {
			printf("[io_uring:sqpoll] SQPOLL thread slept\n");
			return;
		}

		usleep(1000);
	}

	fail_message("io_uring SQPOLL thread did not enter NEED_WAKEUP");
}

static void test_sqpoll_read_write(struct io_uring *ring, int fd,
				   const char *message, size_t message_len,
				   unsigned long long write_user_data,
				   unsigned long long read_user_data)
{
	if (ftruncate(fd, 0) < 0) {
		fail_errno("ftruncate");
	}

	struct io_uring_sqe *sqe = get_sqe(ring, "io_uring SQPOLL write");
	io_uring_prep_write(sqe, fd, message, message_len, 0);
	sqe->user_data = write_user_data;
	submit_one(ring, "io_uring SQPOLL write submit");
	expect_cqe_res_user_data(ring, "io_uring SQPOLL write", write_user_data,
				 message_len);

	char read_buffer[128] = {};
	if (message_len > sizeof(read_buffer)) {
		fail_message("io_uring SQPOLL read buffer is too small");
	}

	sqe = get_sqe(ring, "io_uring SQPOLL read");
	io_uring_prep_read(sqe, fd, read_buffer, message_len, 0);
	sqe->user_data = read_user_data;
	submit_one(ring, "io_uring SQPOLL read submit");
	expect_cqe_res_user_data(ring, "io_uring SQPOLL read", read_user_data,
				 message_len);

	if (memcmp(read_buffer, message, message_len) != 0) {
		fail_message("io_uring SQPOLL read returned unexpected data");
	}
}

int main(void)
{
	setbuf(stdout, NULL);
	alarm(30);
	printf("[io_uring:sqpoll] starting\n");

	struct io_uring ring;
	init_sqpoll_ring(&ring);

	int fd = open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR, 0600);
	if (fd < 0) {
		fail_errno("open");
	}

	test_sqpoll_read_write(&ring, fd, SQPOLL_MESSAGE,
			       sizeof(SQPOLL_MESSAGE), SQPOLL_WRITE_USER_DATA,
			       SQPOLL_READ_USER_DATA);
	printf("[io_uring:sqpoll] file read/write verified\n");

	wait_for_sqpoll_sleep(&ring);
	test_sqpoll_read_write(&ring, fd, SQPOLL_WAKE_MESSAGE,
			       sizeof(SQPOLL_WAKE_MESSAGE),
			       SQPOLL_WAKE_WRITE_USER_DATA,
			       SQPOLL_WAKE_READ_USER_DATA);
	printf("[io_uring:sqpoll] wakeup read/write verified\n");

	if (close(fd) < 0) {
		fail_errno("close");
	}
	if (unlink(TEST_FILE) < 0) {
		fail_errno("unlink");
	}
	io_uring_queue_exit(&ring);

	printf("[io_uring:sqpoll] PASS\n");
	return EXIT_SUCCESS;
}
