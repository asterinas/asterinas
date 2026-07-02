// SPDX-License-Identifier: MPL-2.0

#include "pseudo_file_create.h"

FN_TEST(special_fds)
{
	int udp_socket = TEST_SUCC(socket(AF_INET, SOCK_DGRAM, 0));

	struct fd_fallocate_case {
		int fd;
		int expected_errno;
	};

	struct fd_fallocate_case cases[] = {
		{ pipe_1[0], EBADF },	{ pipe_1[1], ESPIPE },
		{ pipe_2[0], EBADF },	{ pipe_2[1], ESPIPE },
		{ sock[0], ENODEV },	{ sock[1], ENODEV },
		{ epoll_fd, ENODEV },	{ event_fd, ENODEV },
		{ timer_fd, ENODEV },	{ signal_fd, ENODEV },
		{ inotify_fd, EBADF },	{ pid_fd, ENODEV },
		{ mem_fd, 0 },		{ ns_uts_fd, EBADF },
		{ udp_socket, ENODEV },
	};

	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		if (cases[i].expected_errno == 0) {
			TEST_SUCC(fallocate(cases[i].fd, 0, 0, 10));
		} else {
			TEST_ERRNO(fallocate(cases[i].fd, 0, 0, 10),
				   cases[i].expected_errno);
		}
	}

	TEST_SUCC(close(udp_socket));
}
END_TEST()

#include "pseudo_file_cleanup.h"
