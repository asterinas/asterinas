// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <fcntl.h>
#include <linux/capability.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

#define TRUSTED_XATTR_NAME "trusted.capability_lsm_test"

static void read_cap_data(struct __user_cap_data_struct cap_data[2])
{
	struct __user_cap_header_struct cap_header = {
		.version = _LINUX_CAPABILITY_VERSION_3,
		.pid = 0,
	};

	CHECK(syscall(SYS_capget, &cap_header, cap_data));
}

static void drop_cap_sys_admin(void)
{
	struct __user_cap_data_struct cap_data[2] = {};

	read_cap_data(cap_data);
	cap_data[0].effective &= ~(1U << CAP_SYS_ADMIN);
	cap_data[0].permitted &= ~(1U << CAP_SYS_ADMIN);
	cap_data[0].inheritable &= ~(1U << CAP_SYS_ADMIN);
	CHECK(syscall(SYS_capset,
		      &(struct __user_cap_header_struct){
			      .version = _LINUX_CAPABILITY_VERSION_3,
			      .pid = 0,
		      },
		      cap_data));
}

FN_TEST(trusted_xattr_requires_cap_sys_admin)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char file_template[] = "/tmp/trusted_xattrXXXXXX";
		const char initial_value[] = "secret";
		char value_buf[32] = {};
		char list_buf[128] = {};
		int file;
		ssize_t list_len;

		file = CHECK(mkstemp(file_template));

		CHECK(setxattr(file_template, TRUSTED_XATTR_NAME, initial_value,
			       sizeof(initial_value), 0));
		CHECK_WITH(getxattr(file_template, TRUSTED_XATTR_NAME,
				    value_buf, sizeof(value_buf)),
			   _ret == (ssize_t)sizeof(initial_value) &&
				   memcmp(value_buf, initial_value,
					  sizeof(initial_value)) == 0);

		list_len = CHECK(
			listxattr(file_template, list_buf, sizeof(list_buf)));
		CHECK_WITH(memmem(list_buf, list_len, TRUSTED_XATTR_NAME,
				  sizeof(TRUSTED_XATTR_NAME)),
			   _ret != NULL);

		drop_cap_sys_admin();

		errno = 0;
		CHECK_WITH(setxattr(file_template, TRUSTED_XATTR_NAME, "other",
				    sizeof("other"), 0),
			   _ret == -1 && errno == EPERM);
		errno = 0;
		CHECK_WITH(getxattr(file_template, TRUSTED_XATTR_NAME,
				    value_buf, sizeof(value_buf)),
			   _ret == -1 && errno == ENODATA);
		list_len = CHECK(
			listxattr(file_template, list_buf, sizeof(list_buf)));
		CHECK_WITH(memmem(list_buf, list_len, TRUSTED_XATTR_NAME,
				  sizeof(TRUSTED_XATTR_NAME)),
			   _ret == NULL);
		errno = 0;
		CHECK_WITH(removexattr(file_template, TRUSTED_XATTR_NAME),
			   _ret == -1 && errno == EPERM);

		CHECK(close(file));
		CHECK(unlink(file_template));
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()
