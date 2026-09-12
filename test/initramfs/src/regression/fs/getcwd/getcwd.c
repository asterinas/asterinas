// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <limits.h>
#include <sched.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define PARENT_MOUNT "/tmp/getcwd_parent"
#define CHILD_MOUNT "/tmp/getcwd_parent/child"
#define CWD_PATH "/tmp/getcwd_parent/child/cwd"
#define UNREACHABLE_PREFIX "(unreachable)"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret == 0 || errno == EEXIST);
}

FN_TEST(getcwd_small_buffer_returns_erange)
{
	char small[1];
	TEST_ERRNO(getcwd(small, 1), ERANGE);
}
END_TEST()

FN_TEST(getcwd_after_detaching_parent_with_child)
{
	TEST_SUCC(unshare(CLONE_NEWNS));

	ensure_dir(PARENT_MOUNT);
	TEST_SUCC(mount("none", PARENT_MOUNT, "tmpfs", 0, NULL));
	ensure_dir(CHILD_MOUNT);
	TEST_SUCC(mount("none", CHILD_MOUNT, "tmpfs", 0, NULL));
	ensure_dir(CWD_PATH);
	TEST_SUCC(chdir(CWD_PATH));

	TEST_SUCC(umount2(PARENT_MOUNT, MNT_DETACH));

	char cwd[PATH_MAX];
	TEST_RES(syscall(SYS_getcwd, cwd, sizeof(cwd)),
		 strncmp(cwd, UNREACHABLE_PREFIX, strlen(UNREACHABLE_PREFIX)) ==
			 0);

	TEST_SUCC(chdir("/"));
	TEST_SUCC(rmdir(PARENT_MOUNT));
}
END_TEST()
