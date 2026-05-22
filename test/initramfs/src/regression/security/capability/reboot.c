// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/capability.h>
#include <linux/reboot.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static void read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	CHECK(syscall(SYS_capget, &cap_header, cap_data));
}

static void drop_cap_sys_boot(void)
{
	struct __user_cap_data_struct cap_data[2] = {};

	read_cap_data(cap_data);
	cap_data[0].effective &= ~(1U << CAP_SYS_BOOT);
	cap_data[0].permitted &= ~(1U << CAP_SYS_BOOT);
	cap_data[0].inheritable &= ~(1U << CAP_SYS_BOOT);
	CHECK(syscall(SYS_capset,
		      &(struct __user_cap_header_struct){
			      .version = _LINUX_CAPABILITY_VERSION_3,
			      .pid = 0,
		      },
		      cap_data));
}

FN_TEST(reboot_requires_cap_sys_boot)
{
	pid_t pid;
	int status;

	TEST_ERRNO(syscall(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
			   0xdeadbeefU, 0),
		   EINVAL);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		errno = 0;
		drop_cap_sys_boot();
		CHECK_WITH(syscall(SYS_reboot, LINUX_REBOOT_MAGIC1,
				   LINUX_REBOOT_MAGIC2, 0xdeadbeefU, 0),
			   _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
