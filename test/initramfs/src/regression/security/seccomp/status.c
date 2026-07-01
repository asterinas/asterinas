// SPDX-License-Identifier: MPL-2.0

// Verifies that `/proc/[pid]/status` reports the thread's seccomp state, the
// way userspace tools (e.g. systemd, container introspection) observe it:
//
//   NoNewPrivs:      <0|1>
//   Seccomp:         <0 disabled | 1 strict | 2 filter>
//   Seccomp_filters: <number of installed filters>

#define _GNU_SOURCE

#include "../../common/test.h"
#include <fcntl.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stdint.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

// Reads an integer-valued field such as "Seccomp" from /proc/self/status. The
// read itself must be permitted by any installed filter, so the tests below use
// allow-all filters.
static long read_status_field(const char *name)
{
	char buf[8192];
	int fd = CHECK(open("/proc/self/status", O_RDONLY));
	ssize_t len = CHECK(read(fd, buf, sizeof(buf) - 1));
	CHECK(close(fd));
	buf[len] = '\0';

	char key[64];
	int klen = snprintf(key, sizeof(key), "\n%s:", name);
	char *pos = strstr(buf, key);
	if (pos == NULL) {
		return -1;
	}
	pos += klen;
	while (*pos == ' ' || *pos == '\t')
		pos++;
	return atol(pos);
}

static void install_allow_filter(void)
{
	struct sock_filter filter[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = sizeof(filter) / sizeof(filter[0]),
		.filter = filter,
	};

	CHECK(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog));
}

FN_TEST(status_reflects_seccomp_state)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(read_status_field("Seccomp"), _ret == 0);
		CHECK_WITH(read_status_field("NoNewPrivs"), _ret == 0);
		CHECK_WITH(read_status_field("Seccomp_filters"), _ret == 0);

		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK_WITH(read_status_field("NoNewPrivs"), _ret == 1);

		install_allow_filter();
		CHECK_WITH(read_status_field("Seccomp"),
			   _ret == SECCOMP_MODE_FILTER);
		CHECK_WITH(read_status_field("Seccomp_filters"), _ret == 1);

		install_allow_filter();
		CHECK_WITH(read_status_field("Seccomp_filters"), _ret == 2);

		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
