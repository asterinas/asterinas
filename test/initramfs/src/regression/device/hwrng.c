// SPDX-License-Identifier: MPL-2.0

/*
 * Regression tests for `/dev/hwrng`.
 *
 * This test file covers device identity, open-mode behavior, blocking and
 * nonblocking reads, writes, poll, and basic data sanity checks.
 */

#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../common/test.h"

#define HWRNG_DEVICE "/dev/hwrng"
#define HWRNG_MAJOR 10
#define HWRNG_MINOR 183
#define HWRNG_CONCURRENT_READERS 8
#define HWRNG_CONCURRENT_READ_SIZE 256
#define HWRNG_PRIME_READ_SIZE 1
#define HWRNG_SHORT_READ_SIZE 64
#define HWRNG_LARGE_READ_SIZE (16 * 1024)
#define HWRNG_NONBLOCK_MAX_ATTEMPTS 128
#define HWRNG_SMOKE_READ_SIZE 4096
#define HWRNG_POLL_TIMEOUT_MS 1000

static void exit_if_hwrng_is_unavailable(void);
static void run_concurrent_reader(int pipe_fd);
static bool is_all_zero(const uint8_t *buf, size_t len);
static bool is_all_ff(const uint8_t *buf, size_t len);
static bool is_repeating_u32_pattern(const uint8_t *buf, size_t len);
static bool
has_duplicate_buffer(const uint8_t bufs[][HWRNG_CONCURRENT_READ_SIZE],
		     size_t buf_count);
static size_t count_set_bits(const uint8_t *buf, size_t len);

FN_SETUP(check_hwrng_availability)
{
	exit_if_hwrng_is_unavailable();
}
END_SETUP()

FN_TEST(hwrng_has_correct_char_dev_id)
{
	struct stat st;
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(fstat(fd, &st), S_ISCHR(st.st_mode) &&
					 major(st.st_rdev) == HWRNG_MAJOR &&
					 minor(st.st_rdev) == HWRNG_MINOR);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_has_linux_compatible_mode)
{
	struct stat st;
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(fstat(fd, &st), (st.st_mode & 0777) == 0600);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_concurrent_readers_get_distinct_data)
{
	uint8_t bufs[HWRNG_CONCURRENT_READERS][HWRNG_CONCURRENT_READ_SIZE] = {
		0
	};
	pid_t pids[HWRNG_CONCURRENT_READERS] = { 0 };
	int pipe_fds[HWRNG_CONCURRENT_READERS][2];

	for (size_t i = 0; i < HWRNG_CONCURRENT_READERS; ++i) {
		TEST_SUCC(pipe(pipe_fds[i]));

		pids[i] = TEST_SUCC(fork());
		if (pids[i] == 0) {
			CHECK(close(pipe_fds[i][0]));
			run_concurrent_reader(pipe_fds[i][1]);
		}

		TEST_SUCC(close(pipe_fds[i][1]));
	}

	for (size_t i = 0; i < HWRNG_CONCURRENT_READERS; ++i) {
		int status;

		TEST_RES(read(pipe_fds[i][0], bufs[i], sizeof(bufs[i])),
			 _ret == sizeof(bufs[i]));
		TEST_SUCC(close(pipe_fds[i][0]));

		TEST_RES(waitpid(pids[i], &status, 0),
			 _ret == pids[i] && WIFEXITED(status) &&
				 WEXITSTATUS(status) == EXIT_SUCCESS);
		TEST_RES(is_all_zero(bufs[i], sizeof(bufs[i])), _ret == 0);
		TEST_RES(is_all_ff(bufs[i], sizeof(bufs[i])), _ret == 0);
		TEST_RES(is_repeating_u32_pattern(bufs[i], sizeof(bufs[i])),
			 _ret == 0);
	}

	TEST_RES(has_duplicate_buffer(bufs, HWRNG_CONCURRENT_READERS),
		 _ret == 0);
}
END_TEST()

FN_TEST(hwrng_short_read)
{
	uint8_t buf[HWRNG_SHORT_READ_SIZE] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_read_with_zero_count)
{
	uint8_t buf[1] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(read(fd, buf, 0), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

/* A large blocking read should span internal refills and still complete fully. */
FN_TEST(hwrng_large_read)
{
	uint8_t buf[HWRNG_LARGE_READ_SIZE] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));
	TEST_RES(is_all_zero(buf, sizeof(buf)), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_write_on_ro_ebadf)
{
	uint8_t buf[16] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_write_on_wo_einval)
{
	/*
	 * FIXME: Linux rejects `O_WRONLY` and `O_RDWR` in `rng_dev_open()`
	 * with `EINVAL`. Asterinas does not pass the access mode to
	 * `Device::open()` yet, so the open succeeds and the later write fails
	 * with `EBADF` instead.
	 */
#ifdef __asterinas__
	uint8_t buf[16] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_WRONLY));

	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(close(fd));
#else
	TEST_ERRNO(open(HWRNG_DEVICE, O_WRONLY), EINVAL);
#endif
}
END_TEST()

FN_TEST(hwrng_write_on_rw_einval)
{
	/* FIXME: See `hwrng_write_on_wo_einval`. */
#ifdef __asterinas__
	uint8_t buf[16] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDWR));

	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(close(fd));
#else
	TEST_ERRNO(open(HWRNG_DEVICE, O_RDWR), EINVAL);
#endif
}
END_TEST()

/* Linux reports `/dev/hwrng` as always ready to `poll(2)`. */
FN_TEST(hwrng_poll_reports_in_and_out)
{
	uint8_t buf[HWRNG_SHORT_READ_SIZE] = { 0 };
	struct pollfd pfd;
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, HWRNG_POLL_TIMEOUT_MS), _ret == 1);
	TEST_RES(pfd.revents, _ret == (POLLIN | POLLOUT));
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_lseek_is_noop_and_preserves_readability)
{
	uint8_t buf[HWRNG_SHORT_READ_SIZE] = { 0 };
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	/*
	 * Linux binds `/dev/hwrng` to `noop_llseek`, so `lseek(2)` succeeds
	 * without introducing offset-aware reads.
	 * Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L287>.
	 */
	TEST_RES(lseek(fd, 0, SEEK_SET), _ret == 0);
	TEST_RES(lseek(fd, 123, SEEK_CUR), _ret == 0);
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));

	TEST_SUCC(close(fd));
}
END_TEST()

/*
 * `virtio-rng` cannot make the nonblocking outcome deterministic. Draining
 * the cache immediately submits the next request, and a following
 * nonblocking read races the completion IRQ. This test accepts either
 * immediate data or `EAGAIN`.
 */
FN_TEST(hwrng_nonblock_eagain_or_immediate_data)
{
	uint8_t drain_buf[HWRNG_LARGE_READ_SIZE] = { 0 };
	uint8_t nonblock_buf[HWRNG_LARGE_READ_SIZE] = { 0 };
	bool saw_eagain = false;
	bool saw_immediate_data = false;

	for (size_t attempt = 0; attempt < HWRNG_NONBLOCK_MAX_ATTEMPTS;
	     ++attempt) {
		ssize_t nonblock_ret;
		int fd;

		fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));
		TEST_RES(read(fd, drain_buf, sizeof(drain_buf)),
			 _ret == sizeof(drain_buf));
		TEST_SUCC(close(fd));

		fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY | O_NONBLOCK));

		errno = 0;
		nonblock_ret = read(fd, nonblock_buf, sizeof(nonblock_buf));
		if (nonblock_ret == -1 && errno == EAGAIN) {
			saw_eagain = true;
			TEST_SUCC(close(fd));
			break;
		}

		TEST_RES(nonblock_ret, _ret > 0);
		saw_immediate_data = true;

		TEST_SUCC(close(fd));
	}

	TEST_RES(saw_eagain || saw_immediate_data, _ret);
}
END_TEST()

FN_TEST(hwrng_nonblock_succeeds_after_cache_prime)
{
	uint8_t prime_buf[HWRNG_PRIME_READ_SIZE] = { 0 };
	uint8_t buf[HWRNG_SHORT_READ_SIZE] = { 0 };
	int fd;

	fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));
	TEST_RES(read(fd, prime_buf, sizeof(prime_buf)),
		 _ret == sizeof(prime_buf));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY | O_NONBLOCK));
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_small_read_boundaries)
{
	const size_t sizes[] = { 1, 3, 7 };
	uint8_t buf[7] = { 0 };
	size_t total_read = 0;
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	while (total_read < HWRNG_LARGE_READ_SIZE) {
		for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); ++i) {
			memset(buf, 0, sizeof(buf));
			TEST_RES(read(fd, buf, sizes[i]),
				 _ret == (ssize_t)sizes[i]);
			total_read += sizes[i];
		}
	}
	TEST_RES(total_read, _ret >= HWRNG_LARGE_READ_SIZE);

	TEST_SUCC(close(fd));
}
END_TEST()

/*
 * This is a smoke test for obvious DMA or cache-handling bugs. It is not a
 * statistical randomness test.
 */
FN_TEST(hwrng_smoke_basic_content_checks)
{
	uint8_t buf[HWRNG_SMOKE_READ_SIZE] = { 0 };
	size_t set_bits;
	int fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));
	TEST_RES(is_all_zero(buf, sizeof(buf)), _ret == 0);
	TEST_RES(is_all_ff(buf, sizeof(buf)), _ret == 0);
	TEST_RES(is_repeating_u32_pattern(buf, sizeof(buf)), _ret == 0);

	set_bits = count_set_bits(buf, sizeof(buf));
	TEST_RES(set_bits, _ret >= sizeof(buf) * CHAR_BIT * 40 / 100 &&
				   _ret <= sizeof(buf) * CHAR_BIT * 60 / 100);

	TEST_SUCC(close(fd));
}
END_TEST()

static void exit_if_hwrng_is_unavailable(void)
{
	int fd;
	uint8_t buf[HWRNG_SHORT_READ_SIZE] = { 0 };

	fd = open(HWRNG_DEVICE, O_RDONLY);
	if (fd < 0) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr, "hwrng tests skipped: %s (%s)\n",
				HWRNG_DEVICE, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr, "fatal error: %s: open('%s') failed: %s\n",
			__func__, HWRNG_DEVICE, strerror(errno));
		exit(EXIT_FAILURE);
	}

	if (read(fd, buf, sizeof(buf)) == -1) {
		int saved_errno = errno;

		if (saved_errno == ENODEV || saved_errno == EIO ||
		    saved_errno == ENXIO) {
			fprintf(stderr, "hwrng tests skipped: read('%s'): %s\n",
				HWRNG_DEVICE, strerror(saved_errno));
			CHECK(close(fd));
			exit(EXIT_SUCCESS);
		}

		fprintf(stderr, "fatal error: %s: read('%s') failed: %s\n",
			__func__, HWRNG_DEVICE, strerror(saved_errno));
		CHECK(close(fd));
		exit(EXIT_FAILURE);
	}

	CHECK(close(fd));
}

static void run_concurrent_reader(int pipe_fd)
{
	uint8_t buf[HWRNG_CONCURRENT_READ_SIZE] = { 0 };
	int fd = CHECK(open(HWRNG_DEVICE, O_RDONLY));

	CHECK_WITH(read(fd, buf, sizeof(buf)), _ret == sizeof(buf));
	CHECK_WITH(write(pipe_fd, buf, sizeof(buf)), _ret == sizeof(buf));

	CHECK(close(fd));
	CHECK(close(pipe_fd));
	_exit(EXIT_SUCCESS);
}

static bool is_all_zero(const uint8_t *buf, size_t len)
{
	for (size_t i = 0; i < len; ++i) {
		if (buf[i] != 0) {
			return false;
		}
	}

	return true;
}

static bool is_all_ff(const uint8_t *buf, size_t len)
{
	for (size_t i = 0; i < len; ++i) {
		if (buf[i] != UINT8_MAX) {
			return false;
		}
	}

	return true;
}

static bool is_repeating_u32_pattern(const uint8_t *buf, size_t len)
{
	if (len < sizeof(uint32_t) || len % sizeof(uint32_t) != 0) {
		return false;
	}

	for (size_t i = sizeof(uint32_t); i < len; ++i) {
		if (buf[i] != buf[i % sizeof(uint32_t)]) {
			return false;
		}
	}

	return true;
}

static bool
has_duplicate_buffer(const uint8_t bufs[][HWRNG_CONCURRENT_READ_SIZE],
		     size_t buf_count)
{
	for (size_t i = 0; i < buf_count; ++i) {
		for (size_t j = i + 1; j < buf_count; ++j) {
			if (memcmp(bufs[i], bufs[j],
				   HWRNG_CONCURRENT_READ_SIZE) == 0) {
				return true;
			}
		}
	}

	return false;
}

static size_t count_set_bits(const uint8_t *buf, size_t len)
{
	size_t total = 0;

	for (size_t i = 0; i < len; ++i) {
		total += __builtin_popcount(buf[i]);
	}

	return total;
}
