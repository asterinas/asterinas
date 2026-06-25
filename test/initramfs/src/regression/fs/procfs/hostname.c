// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <string.h>
#include <sys/utsname.h>
#include <unistd.h>

#include "../../common/test.h"

#define HOSTNAME_PATH "/proc/sys/kernel/hostname"

FN_TEST(proc_sys_kernel_hostname_matches_uname)
{
	struct utsname uts;
	char hostname[256] = { 0 };
	int fd = TEST_SUCC(open(HOSTNAME_PATH, O_RDONLY));
	ssize_t bytes_read =
		TEST_RES(read(fd, hostname, sizeof(hostname) - 1), _ret > 0);

	TEST_SUCC(uname(&uts));
	TEST_RES(hostname[bytes_read - 1], _ret == '\n');
	hostname[bytes_read - 1] = '\0';
	TEST_RES(strcmp(hostname, uts.nodename), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()
