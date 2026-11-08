// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <sched.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define MOUNT_SOURCE "/tmp/cap_mount_source"
#define MOUNT_TARGET "/tmp/cap_mount_target"

FN_SETUP(init)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));
	CHECK(mkdir(MOUNT_SOURCE, 0755));
	CHECK(mkdir(MOUNT_TARGET, 0755));
	CHECK(mount("tmpfs", MOUNT_SOURCE, "tmpfs", 0, NULL));
}
END_SETUP()

static void check_mount_denied(void)
{
	CHECK_WITH(mount("tmpfs", MOUNT_TARGET, "tmpfs", 0, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount(NULL, MOUNT_SOURCE, NULL, MS_REMOUNT | MS_RDONLY,
			 NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount(NULL, MOUNT_SOURCE, NULL,
			 MS_REMOUNT | MS_BIND | MS_RDONLY, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount(MOUNT_SOURCE, MOUNT_TARGET, NULL, MS_BIND, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount(MOUNT_SOURCE, MOUNT_TARGET, NULL, MS_MOVE, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount(NULL, MOUNT_SOURCE, NULL, MS_SHARED, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(umount(MOUNT_SOURCE), _ret == -1 && errno == EPERM);
	CHECK_WITH(umount2(MOUNT_SOURCE, MNT_DETACH),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(syscall(SYS_pivot_root, MOUNT_SOURCE, MOUNT_SOURCE),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(syscall(SYS_pivot_root, NULL, NULL),
		   _ret == -1 && errno == EPERM);
	CHECK_WITH(mount("tmpfs", "/tmp/cap_mount_missing", "tmpfs", 0, NULL),
		   _ret == -1 && errno == ENOENT);
	CHECK_WITH(umount("/tmp/cap_mount_missing"),
		   _ret == -1 && errno == ENOENT);
}

FN_TEST(mount_requires_cap_sys_admin_even_for_root)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		drop_capability(CAP_SYS_ADMIN);
		check_mount_denied();
		_exit(EXIT_SUCCESS);
	}
	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(unprivileged_user_cannot_modify_root_mounts)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgid(1000));
		CHECK(setuid(1000));
		check_mount_denied();
		_exit(EXIT_SUCCESS);
	}
	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MOUNT_SOURCE));
	CHECK(rmdir(MOUNT_SOURCE));
	CHECK(rmdir(MOUNT_TARGET));
}
END_SETUP()
