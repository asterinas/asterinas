// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/capability.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#define CAP_MASK(capability) (1ULL << (capability))

static void read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	CHECK(syscall(SYS_capget, &cap_header, cap_data));
}

static int __write_cap_data(const struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	return syscall(SYS_capset, &cap_header, cap_data);
}

static void write_cap_data(const struct __user_cap_data_struct cap_data[2])
{
	CHECK(__write_cap_data(cap_data));
}

static void read_proc_capbnd(uint64_t *capbnd)
{
	FILE *status_file;
	char line[256];
	int found_capbnd = 0;

	status_file = CHECK_WITH(fopen("/proc/self/status", "r"), _ret != NULL);

	while (fgets(line, sizeof(line), status_file) != NULL) {
		unsigned long long value;

		if (sscanf(line, "CapBnd:\t%llx", &value) != 1) {
			continue;
		}

		*capbnd = (uint64_t)value;
		found_capbnd = 1;
		break;
	}

	CHECK(fclose(status_file));
	CHECK_WITH(found_capbnd, _ret == 1);
}

FN_TEST(read_initial_bounding_set)
{
	TEST_RES(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);
}
END_TEST()

FN_TEST(read_invalid_capability)
{
	TEST_ERRNO(prctl(PR_CAPBSET_READ, CAP_LAST_CAP + 1), EINVAL);
}
END_TEST()

FN_TEST(drop_bounding_cap)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		uint64_t initial_capbnd;
		uint64_t capbnd;
		struct __user_cap_data_struct cap_data[2] = {};

		// Test 1: `PR_CAPBSET_DROP` works.

		CHECK_WITH(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);
		read_proc_capbnd(&initial_capbnd);

		CHECK(prctl(PR_CAPBSET_DROP, CAP_SYS_ADMIN));
		CHECK_WITH(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 0);
		read_proc_capbnd(&capbnd);
		CHECK_WITH(capbnd,
			   _ret == (initial_capbnd & ~CAP_MASK(CAP_SYS_ADMIN)));

		// Test 2: New inheritable capabilities must be bounding capabilities.

		read_cap_data(cap_data);
		cap_data[0].inheritable &= ~(1U << CAP_SYS_ADMIN);
		write_cap_data(cap_data);

		cap_data[0].inheritable |= (1U << CAP_SYS_ADMIN);
		CHECK_WITH(__write_cap_data(cap_data),
			   _ret == -1 && errno == EPERM);

		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);
}
END_TEST()

FN_TEST(drop_bounding_cap_without_cap_setpcap)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct __user_cap_data_struct cap_data[2] = {};

		read_cap_data(cap_data);
		cap_data[0].effective &= ~(1U << CAP_SETPCAP);
		write_cap_data(cap_data);

		CHECK_WITH(prctl(PR_CAPBSET_DROP, CAP_SYS_ADMIN),
			   _ret == -1 && errno == EPERM);
		CHECK_WITH(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);

		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
	TEST_RES(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);
}
END_TEST()
