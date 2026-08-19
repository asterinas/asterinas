// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/capability.h"

FN_TEST(profile_denies_sys_chroot)
{
	char dir[] = "/tmp/apparmor-chroot-XXXXXX";

	TEST_RES(mkdtemp(dir), _ret != NULL);
	TEST_ERRNO(chroot(dir), EPERM);
	TEST_RES(rmdir(dir), _ret == 0);
}
END_TEST()

FN_TEST(profile_denies_sys_chroot_in_forked_child)
{
	char dir[] = "/tmp/apparmor-chroot-child-XXXXXX";

	TEST_RES(mkdtemp(dir), _ret != NULL);
	pid_t child_pid = TEST_SUCC(fork());

	if (child_pid == 0) {
		errno = 0;
		if (chroot(dir) != -1 || errno != EPERM) {
			_exit(EXIT_FAILURE);
		}
		_exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(waitpid(child_pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(rmdir(dir), _ret == 0);
}
END_TEST()

FN_TEST(profile_allows_cap_chown)
{
	char file_path[] = "/tmp/apparmor-chown-XXXXXX";
	int fd = TEST_SUCC(mkstemp(file_path));

	TEST_SUCC(fchown(fd, 1, 1));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(file_path));
}
END_TEST()
