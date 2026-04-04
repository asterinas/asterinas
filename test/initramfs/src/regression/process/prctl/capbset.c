// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/capability.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#define CAPS_ALL 0x000001ffffffffffULL

static int read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	return syscall(SYS_capget, &cap_header, cap_data);
}

static int write_cap_data(const struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	return syscall(SYS_capset, &cap_header, cap_data);
}

static int read_proc_capbnd(uint64_t *capbnd)
{
	FILE *status_file;
	char line[256];

	status_file = fopen("/proc/self/status", "r");
	if (status_file == NULL) {
		return -1;
	}

	while (fgets(line, sizeof(line), status_file) != NULL) {
		unsigned long long value;

		if (strncmp(line, "CapBnd:\t", strlen("CapBnd:\t")) != 0) {
			continue;
		}

		if (sscanf(line + strlen("CapBnd:\t"), "%llx", &value) != 1) {
			fclose(status_file);
			errno = EINVAL;
			return -1;
		}

		fclose(status_file);
		*capbnd = (uint64_t)value;
		return 0;
	}

	fclose(status_file);
	errno = ENOENT;
	return -1;
}

static int expect_child_exit_success(int (*child_fn)(void))
{
	pid_t pid;
	int status;

	pid = fork();
	if (pid < 0) {
		return -1;
	}

	if (pid == 0) {
		_exit(child_fn() == 0 ? 0 : 1);
	}

	if (waitpid(pid, &status, 0) != pid) {
		return -1;
	}

	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		errno = ECHILD;
		return -1;
	}

	errno = 0;
	return 0;
}

static int drop_and_verify_bounding_cap(void)
{
	struct __user_cap_data_struct cap_data[2] = {};
	uint64_t capbnd;
	uint64_t expected_capbnd = CAPS_ALL & ~(1ULL << CAP_SYS_ADMIN);

	if (prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN) != 1) {
		return -1;
	}

	if (prctl(PR_CAPBSET_DROP, CAP_SYS_ADMIN) != 0) {
		return -1;
	}

	if (prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN) != 0) {
		errno = EINVAL;
		return -1;
	}

	if (read_proc_capbnd(&capbnd) < 0) {
		return -1;
	}

	if (capbnd != expected_capbnd) {
		errno = EINVAL;
		return -1;
	}

	if (read_cap_data(cap_data) < 0) {
		return -1;
	}

	cap_data[0].inheritable |= 1U << CAP_SYS_ADMIN;
	if (write_cap_data(cap_data) != -1 || errno != EPERM) {
		errno = EINVAL;
		return -1;
	}

	errno = 0;
	return 0;
}

static int drop_bounding_cap_without_setpcap(void)
{
	struct __user_cap_data_struct cap_data[2] = {};

	if (read_cap_data(cap_data) < 0) {
		return -1;
	}

	cap_data[0].effective &= ~(1U << CAP_SETPCAP);
	if (write_cap_data(cap_data) < 0) {
		return -1;
	}

	if (prctl(PR_CAPBSET_DROP, CAP_SYS_ADMIN) != -1 || errno != EPERM) {
		errno = EINVAL;
		return -1;
	}

	errno = 0;
	return 0;
}

FN_TEST(prctl_capbset)
{
	TEST_RES(prctl(PR_CAPBSET_READ, CAP_SYS_ADMIN), _ret == 1);
	TEST_ERRNO(prctl(PR_CAPBSET_READ, CAP_CHECKPOINT_RESTORE + 1), EINVAL);
	TEST_SUCC(expect_child_exit_success(drop_and_verify_bounding_cap));
	TEST_SUCC(expect_child_exit_success(drop_bounding_cap_without_setpcap));
}
END_TEST()
