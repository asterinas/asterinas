// SPDX-License-Identifier: MPL-2.0

/* Control-device ABI: version negotiation, capability checks, and user-copy
 * boundaries. These tests use raw ioctls, not dmsetup. */
#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/dm-ioctl.h>
#include <poll.h>
#include <stdint.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../common/capability.h"
#include "../common/test.h"

#define DM_CONTROL_PATH "/dev/mapper/control"

/* Match the DM ABI of the pinned Linux 6.6 baseline (5078a3a23a1e). */
static const uint32_t kernel_version[3] = { 4, 48, 0 };
static const uint32_t client_version[3] = { 4, 0, UINT32_MAX };

FN_TEST(device_mapper_control_without_events_is_not_ready)
{
	const short masks[] = { POLLIN, POLLOUT, POLLIN | POLLOUT };
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	for (size_t i = 0; i < sizeof(masks) / sizeof(masks[0]); i++) {
		struct pollfd poll_fd = {
			.fd = fd,
			.events = masks[i],
			.revents = -1,
		};

		TEST_RES(poll(&poll_fd, 1, 0), _ret == 0);
		TEST_RES(poll_fd.revents, _ret == 0);
	}
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_version_compatibility)
{
	const struct {
		uint32_t version[3];
		int error;
	} cases[] = {
		{ { 4, 0, 0 }, 0 },	      { { 4, 48, 0 }, 0 },
		{ { 4, 48, UINT32_MAX }, 0 }, { { 3, 0, 0 }, EINVAL },
		{ { 5, 0, 0 }, EINVAL },      { { 4, 49, 0 }, EINVAL },
		{ { 4, 50, 0 }, EINVAL },     { { 4, UINT32_MAX, 0 }, EINVAL },
	};
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		struct dm_ioctl io = {};

		memcpy(io.version, cases[i].version, sizeof(io.version));
		io.data_size = sizeof(io);
		io.data_start = sizeof(io);
		TEST(ioctl(fd, DM_VERSION, &io), cases[i].error,
		     _ret == (cases[i].error ? -1 : 0));
		TEST_RES(memcmp(io.version, kernel_version, sizeof(io.version)),
			 _ret == 0);
	}
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_negotiates_before_command_lookup)
{
	struct dm_ioctl io = {}, before;
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	memcpy(io.version, client_version, sizeof(io.version));
	memcpy(&before, &io, sizeof(io));
	TEST(ioctl(fd, _IOWR(0xfe, 0, struct dm_ioctl), &io), ENOTTY,
	     _ret == -1);
	TEST_RES(memcmp(&io, &before, sizeof(io)), _ret == 0);

	/* An unknown command in the DM namespace still exchanges versions. */
	TEST(ioctl(fd, _IOWR(0xfd, 0xff, struct dm_ioctl), &io), ENOTTY,
	     _ret == -1);
	TEST_RES(memcmp(io.version, kernel_version, sizeof(io.version)),
		 _ret == 0);

	memcpy(io.version, client_version, sizeof(io.version));
	io.version[0] = 5;
	TEST(ioctl(fd, _IOWR(0xfd, 0xff, struct dm_ioctl), &io), EINVAL,
	     _ret == -1);
	TEST_RES(memcmp(io.version, kernel_version, sizeof(io.version)),
		 _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_version_ignores_non_version_fields)
{
	const uint32_t sizes[] = { 0, 312, 1024 * 1024 + 1, UINT32_MAX };
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		struct dm_ioctl io, before;

		/* Include unsupported flags and a name without a terminating NUL. */
		memset(&io, 0xff, sizeof(io));
		memcpy(io.version, client_version, sizeof(io.version));
		io.data_size = sizes[i];
		io.data_start = i;
		memcpy(&before, &io, sizeof(io));
		TEST_RES(ioctl(fd, DM_VERSION, &io), _ret == 0);
		TEST_RES(memcmp(io.version, kernel_version, sizeof(io.version)),
			 _ret == 0);
		TEST_RES(memcmp((char *)&io + sizeof(io.version),
				(char *)&before + sizeof(io.version),
				sizeof(io) - sizeof(io.version)),
			 _ret == 0);
	}
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_version_only_accesses_version_bytes)
{
	long page_size = CHECK_WITH(sysconf(_SC_PAGESIZE), _ret > 0);
	char *pages =
		CHECK_WITH(mmap(NULL, page_size * 2, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	char *version = pages + page_size - sizeof(kernel_version);
	struct dm_ioctl io = {};
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	memcpy(io.version, client_version, sizeof(io.version));
	io.data_size = sizeof(io);
	io.data_start = sizeof(io);
	memcpy(version, &io, sizeof(io));

	/* No read beyond version[3] is needed, despite the ioctl size encoding. */
	CHECK(mprotect(pages + page_size, page_size, PROT_NONE));
	const unsigned long commands[] = {
		DM_VERSION,
		_IO(0xfd, DM_VERSION_CMD),
		_IOC(_IOC_READ, 0xfd, DM_VERSION_CMD, 1),
		_IOC(_IOC_WRITE, 0xfd, DM_VERSION_CMD, 0x3fff),
	};
	for (size_t i = 0; i < sizeof(commands) / sizeof(commands[0]); i++) {
		memcpy(version, client_version, sizeof(client_version));
		TEST_RES(ioctl(fd, commands[i], version), _ret == 0);
		TEST_RES(memcmp(version, kernel_version,
				sizeof(kernel_version)),
			 _ret == 0);
	}

	/* Nor may a successful handshake write the readable, read-only tail. */
	CHECK(mprotect(pages + page_size, page_size, PROT_READ));
	memcpy(version, client_version, sizeof(client_version));
	TEST_RES(ioctl(fd, DM_VERSION, version), _ret == 0);
	TEST_RES(memcmp(version, kernel_version, sizeof(kernel_version)),
		 _ret == 0);
	TEST_RES(memcmp(pages + page_size, (char *)&io + sizeof(io.version),
			sizeof(io) - sizeof(io.version)),
		 _ret == 0);

	TEST_SUCC(munmap(pages, page_size * 2));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_version_reports_memory_faults)
{
	long page_size = CHECK_WITH(sysconf(_SC_PAGESIZE), _ret > 0);
	struct dm_ioctl *io =
		CHECK_WITH(mmap(NULL, page_size, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			   _ret != MAP_FAILED);
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));

	io->data_size = sizeof(*io);
	io->data_start = sizeof(*io);
	memcpy(io->version, client_version, sizeof(io->version));
	CHECK(mprotect(io, page_size, PROT_READ));
	TEST(ioctl(fd, DM_VERSION, io), EFAULT, _ret == -1);

	/* A failed version copyout takes precedence over an incompatible major. */
	CHECK(mprotect(io, page_size, PROT_READ | PROT_WRITE));
	io->version[0] = 5;
	CHECK(mprotect(io, page_size, PROT_READ));
	TEST(ioctl(fd, DM_VERSION, io), EFAULT, _ret == -1);

	CHECK(mprotect(io, page_size, PROT_NONE));
	TEST(ioctl(fd, DM_VERSION, io), EFAULT, _ret == -1);
	TEST_SUCC(munmap(io, page_size));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(device_mapper_requires_admin_on_each_ioctl)
{
	int fd = CHECK(open(DM_CONTROL_PATH, O_RDWR | O_CLOEXEC));
	pid_t pid = CHECK(fork());
	int status;

	if (pid == 0) {
		struct dm_ioctl io = {}, before;
		int result, error;

		/* Retain the open fd and root UID, but remove CAP_SYS_ADMIN. */
		CHECK_WITH(geteuid(), _ret == 0);
		drop_capability(CAP_SYS_ADMIN);
		memcpy(io.version, client_version, sizeof(io.version));
		io.data_size = sizeof(io);
		io.data_start = sizeof(io);
		memcpy(&before, &io, sizeof(io));
		errno = 0;
		result = ioctl(fd, DM_VERSION, &io);
		error = errno;
		if (result != -1 || error != EACCES ||
		    memcmp(&io, &before, sizeof(io)) != 0)
			_exit(EXIT_FAILURE);

		/* Reject mutating commands before even accessing their arguments. */
		errno = 0;
		result = ioctl(fd, DM_DEV_CREATE, (void *)UINTPTR_MAX);
		error = errno;
		close(fd);
		_exit(result == -1 && error == EACCES ? EXIT_SUCCESS :
							EXIT_FAILURE);
	}
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_SUCC(close(fd));
}
END_TEST()
