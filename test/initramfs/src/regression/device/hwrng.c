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

FN_TEST(hwrng_basics)
{
	uint8_t buf1[64] = { 0 };
	uint8_t buf2[64] = { 0 };
	uint8_t write_buf[16] = { 0 };
	struct stat st;
	struct pollfd pfd;
	int fd;
	int is_unavailable;

	fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	is_unavailable =
		(read(fd, buf1, sizeof(buf1)) == -1 && errno == ENODEV);
	if (is_unavailable) {
		(void)close(fd);
	}
	SKIP_TEST_IF(is_unavailable);

	TEST_RES(fstat(fd, &st), S_ISCHR(st.st_mode) &&
					 major(st.st_rdev) == HWRNG_MAJOR &&
					 minor(st.st_rdev) == HWRNG_MINOR);

	TEST_RES(read(fd, buf1, sizeof(buf1)), _ret == sizeof(buf1));
	TEST_RES(read(fd, buf2, sizeof(buf2)), _ret == sizeof(buf2));
	TEST_RES(memcmp(buf1, buf2, sizeof(buf1)), _ret != 0);

	TEST_ERRNO(write(fd, write_buf, sizeof(write_buf)), EBADF);

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, 1000), _ret == 1);
	TEST_RES(pfd.revents, _ret == (POLLIN | POLLOUT));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(hwrng_write)
{
#ifdef __asterinas__
	int fd;
	uint8_t buf[16] = { 0 };

	// FIXME: Asterinas should reject opening `/dev/hwrng` with either
	// `O_WRONLY` or `O_RDWR`.
	fd = TEST_SUCC(open(HWRNG_DEVICE, O_WRONLY));
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDWR));
	TEST_ERRNO(write(fd, buf, sizeof(buf)), EBADF);
	TEST_SUCC(close(fd));
#else
	TEST_ERRNO(open(HWRNG_DEVICE, O_WRONLY), EINVAL);
	TEST_ERRNO(open(HWRNG_DEVICE, O_RDWR), EINVAL);
#endif
}
END_TEST()
