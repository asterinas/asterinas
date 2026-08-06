// SPDX-License-Identifier: MPL-2.0

#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/fcntl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>
#include "../common/test.h"

#define PAGE_SIZE 4096
#define WAIT_CHILD_ATTEMPTS 150
#define WAIT_CHILD_INTERVAL_US 20000

#ifndef GRND_NONBLOCK
#define GRND_NONBLOCK 0x0001
#endif

#ifndef GRND_RANDOM
#define GRND_RANDOM 0x0002
#endif

#ifndef GRND_INSECURE
#define GRND_INSECURE 0x0004
#endif

struct random_device {
	const char *path;
	unsigned int major;
	unsigned int minor;
};

static const struct random_device random_devices[] = {
	{ "/dev/random", 1, 8 },
	{ "/dev/urandom", 1, 9 },
};

static ssize_t sys_getrandom(void *buf, size_t len, unsigned int flags)
{
	return syscall(SYS_getrandom, buf, len, flags);
}

static int secure_random_is_ready(void)
{
	uint8_t byte;

	errno = 0;
	return sys_getrandom(&byte, sizeof(byte), GRND_NONBLOCK) ==
	       sizeof(byte);
}

static void sigalrm_handler(int sig)
{
	(void)sig;
}

static void child_wait_for_getrandom_signal(int pipe_fd)
{
	struct sigaction action = {
		.sa_handler = sigalrm_handler,
	};
	uint8_t byte;
	ssize_t ret;

	CHECK(sigaction(SIGALRM, &action, NULL));
	CHECK(write(pipe_fd, "", 1));
	CHECK_WITH(alarm(1), _ret == 0);

	errno = 0;
	ret = sys_getrandom(&byte, sizeof(byte), 0);
	if (ret == -1 && errno == EINTR) {
		_exit(EXIT_SUCCESS);
	}
	if (ret == sizeof(byte)) {
		_exit(2);
	}
	_exit(EXIT_FAILURE);
}

static int wait_child_with_timeout(pid_t child, int *status)
{
	for (int i = 0; i < WAIT_CHILD_ATTEMPTS; ++i) {
		pid_t ret = waitpid(child, status, WNOHANG);

		if (ret == child) {
			return 0;
		}
		if (ret < 0) {
			return -1;
		}

		usleep(WAIT_CHILD_INTERVAL_US);
	}

	kill(child, SIGKILL);
	waitpid(child, status, 0);
	errno = ETIMEDOUT;
	return -1;
}

FN_TEST(random_devices_have_correct_char_dev_ids)
{
	for (size_t i = 0;
	     i < sizeof(random_devices) / sizeof(random_devices[0]); i++) {
		const struct random_device *device = &random_devices[i];
		struct stat stat_buf;

		TEST_RES(stat(device->path, &stat_buf),
			 S_ISCHR(stat_buf.st_mode) &&
				 stat_buf.st_rdev ==
					 makedev(device->major, device->minor));
	}
}
END_TEST()

FN_TEST(random_devices_have_linux_compatible_mode)
{
	for (size_t i = 0;
	     i < sizeof(random_devices) / sizeof(random_devices[0]); i++) {
		const struct random_device *device = &random_devices[i];
		struct stat stat_buf;

		TEST_RES(stat(device->path, &stat_buf),
			 (stat_buf.st_mode & 0777) == 0666);
	}
}
END_TEST()

FN_TEST(random_read_fault_boundaries)
{
	int fd;
	char *buf;

	fd = TEST_SUCC(open("/dev/random", O_RDONLY));

	buf = TEST_SUCC(mmap(NULL, PAGE_SIZE * 3, PROT_READ | PROT_WRITE,
			     MAP_ANONYMOUS | MAP_PRIVATE, -1, 0));
	TEST_SUCC(munmap(buf + PAGE_SIZE * 2, PAGE_SIZE));

	// Invalid address
	TEST_ERRNO(read(fd, buf + PAGE_SIZE * 2, PAGE_SIZE), EFAULT);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2, 0), _ret == 0);

	// Valid address, insufficient space
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - 1, PAGE_SIZE), _ret == 1);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE - 1), PAGE_SIZE + 2),
		 _ret == (PAGE_SIZE - 1));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - PAGE_SIZE, PAGE_SIZE + 2),
		 _ret == PAGE_SIZE);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE + 1), PAGE_SIZE + 2),
		 _ret == (PAGE_SIZE + 1));

	// Valid address, sufficient space
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - 1, 1), _ret == 1);
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE - 1), PAGE_SIZE - 2),
		 _ret == (PAGE_SIZE - 2));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - PAGE_SIZE, PAGE_SIZE - 1),
		 _ret == (PAGE_SIZE - 1));
	TEST_RES(read(fd, buf + PAGE_SIZE * 2 - (PAGE_SIZE + 1), PAGE_SIZE),
		 _ret == PAGE_SIZE);

	TEST_SUCC(munmap(buf, PAGE_SIZE * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(getrandom_rejects_unknown_flags)
{
	uint8_t buf[16] = { 0 };

	TEST_ERRNO(sys_getrandom(buf, sizeof(buf), 0x80000000U), EINVAL);
}
END_TEST()

FN_TEST(getrandom_rejects_insecure_random)
{
	uint8_t buf[16] = { 0 };

	TEST_ERRNO(sys_getrandom(buf, sizeof(buf), GRND_INSECURE | GRND_RANDOM),
		   EINVAL);
}
END_TEST()

FN_TEST(getrandom_zero_count_returns_zero)
{
	uint8_t buf[1] = { 0 };

	TEST_RES(sys_getrandom(buf, 0, 0), _ret == 0);
}
END_TEST()

FN_TEST(getrandom_insecure_returns_requested_bytes)
{
	uint8_t buf[16] = { 0 };

	TEST_RES(sys_getrandom(buf, sizeof(buf), GRND_INSECURE),
		 _ret == sizeof(buf));
}
END_TEST()

FN_TEST(getrandom_nonblock_matches_readiness)
{
	uint8_t buf[16] = { 0 };
	ssize_t ret;
	int saved_errno;

	errno = 0;
	ret = sys_getrandom(buf, sizeof(buf), GRND_NONBLOCK);
	saved_errno = errno;
	if (ret == -1) {
		TEST_RES(saved_errno, _ret == EAGAIN);
	} else {
		TEST_RES(ret, _ret == sizeof(buf));
	}
}
END_TEST()

FN_TEST(random_poll_matches_readiness)
{
	struct pollfd pfd;
	int was_ready = secure_random_is_ready();
	int fd = TEST_SUCC(open("/dev/random", O_RDONLY | O_NONBLOCK));

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, 0), _ret == 1);

	if (was_ready) {
		TEST_RES(pfd.revents & POLLIN, _ret != 0);
	} else {
		// The random subsystem may become ready between the probe and poll.
		TEST_RES(pfd.revents & POLLIN,
			 _ret == 0 || secure_random_is_ready());
	}
	TEST_RES(pfd.revents & POLLOUT, _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(urandom_poll_always_readable)
{
	struct pollfd pfd;
	int fd = TEST_SUCC(open("/dev/urandom", O_RDONLY));

	pfd = (struct pollfd){
		.fd = fd,
		.events = POLLIN | POLLOUT,
	};

	TEST_RES(poll(&pfd, 1, 0), _ret == 1);
	TEST_RES(pfd.revents & POLLIN, _ret != 0);
	TEST_RES(pfd.revents & POLLOUT, _ret != 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(blocking_getrandom_is_interruptible_before_readiness)
{
	int pipe_fds[2];
	int status = 0;
	pid_t child;
	char byte;

	// This path is only reachable on boots where the secure RNG is not ready yet.
	SKIP_TEST_IF(secure_random_is_ready());

	TEST_SUCC(pipe(pipe_fds));

	child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(pipe_fds[0]));
		child_wait_for_getrandom_signal(pipe_fds[1]);
	}

	TEST_SUCC(close(pipe_fds[1]));
	TEST_RES(read(pipe_fds[0], &byte, sizeof(byte)), _ret == sizeof(byte));
	TEST_SUCC(close(pipe_fds[0]));

	TEST_SUCC(wait_child_with_timeout(child, &status));
	if (WIFEXITED(status) && WEXITSTATUS(status) == 2) {
		SKIP_TEST_IF(1);
	}
	TEST_RES(status, WIFEXITED(_ret) && WEXITSTATUS(_ret) == EXIT_SUCCESS);
}
END_TEST()
