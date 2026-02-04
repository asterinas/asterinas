// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../common/test.h"

#define HWRNG_DEVICE "/dev/hwrng"
#define HWRNG_MAJOR 10
#define HWRNG_MINOR 183

static int hwrng_fd = -1;

FN_TEST(open_hwrng)
{
	struct stat st;

	hwrng_fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(fstat(hwrng_fd, &st),
		 S_ISCHR(st.st_mode) && major(st.st_rdev) == HWRNG_MAJOR &&
			 minor(st.st_rdev) == HWRNG_MINOR);
}
END_TEST()

FN_TEST(read_hwrng)
{
	uint8_t buf1[64];
	uint8_t buf2[64];

	ssize_t ret = read(hwrng_fd, buf1, sizeof(buf1));

	if (ret < 0) {
		if (errno == ENODEV) {
			fprintf(stderr, "hwrng tests skipped: %s (%s)\n",
				HWRNG_DEVICE, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: read_hwrng: read('%s') failed: %s\n",
			HWRNG_DEVICE, strerror(errno));
		exit(EXIT_FAILURE);
	}

	TEST_RES(read(hwrng_fd, buf2, sizeof(buf2)), _ret == 64);

	TEST_RES(memcmp(buf1, buf2, sizeof(buf1)), _ret != 0);
}
END_TEST()

FN_TEST(write_hwrng)
{
	uint8_t buf[16] = { 0 };

	// TEST_ERRNO(write(hwrng_fd, buf, sizeof(buf)), EBADF);
	int ret = write(hwrng_fd, buf, sizeof(buf));
	if (ret < 0) {
		fprintf(stderr,
			"write_hwrng: write('%s') failed as expected: %s\n",
			HWRNG_DEVICE, strerror(errno));
	} else {
		fprintf(stderr,
			"fatal error: write_hwrng: write('%s') unexpectedly succeeded\n",
			HWRNG_DEVICE);
		exit(EXIT_FAILURE);
	}
}
END_TEST()

FN_TEST(poll_hwrng)
{
	struct pollfd pfd = {
		.fd = hwrng_fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, 1000), _ret == 1);
	TEST_RES(pfd.revents, _ret == (POLLIN | POLLOUT));
}
END_TEST()

FN_TEST(close_hwrng)
{
	TEST_SUCC(close(hwrng_fd));
}
END_TEST()
