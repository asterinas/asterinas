// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <linux/reboot.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

FN_TEST(reboot_requires_cap_sys_boot)
{
	pid_t pid;
	int status;

	TEST_ERRNO(syscall(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
			   0xdeadbeefU, 0),
		   EINVAL);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SYS_BOOT);
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
