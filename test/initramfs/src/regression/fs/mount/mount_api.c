// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#ifndef SYS_FSOPEN
#define SYS_FSOPEN 430
#endif
#ifndef SYS_FSCONFIG
#define SYS_FSCONFIG 431
#endif
#ifndef SYS_FSMOUNT
#define SYS_FSMOUNT 432
#endif
#ifndef SYS_MOVE_MOUNT
#define SYS_MOVE_MOUNT 429
#endif

#ifndef FSCONFIG_SET_FLAG
#define FSCONFIG_SET_FLAG 0
#endif
#ifndef FSCONFIG_SET_STRING
#define FSCONFIG_SET_STRING 1
#endif
#ifndef FSCONFIG_CMD_CREATE
#define FSCONFIG_CMD_CREATE 6
#endif
#ifndef MOVE_MOUNT_F_EMPTY_PATH
#define MOVE_MOUNT_F_EMPTY_PATH 0x00000004
#endif

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret == 0 || errno == EEXIST);
}

static int create_detached_tmpfs(void)
{
	int fs_fd = CHECK(syscall(SYS_FSOPEN, "tmpfs", 0));
	CHECK(syscall(SYS_FSCONFIG, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL, 0));
	int mount_fd = CHECK(syscall(SYS_FSMOUNT, fs_fd, 0, 0));
	CHECK(close(fs_fd));
	return mount_fd;
}

FN_TEST(fsopen_invalid_fsname)
{
	TEST_ERRNO(syscall(SYS_FSOPEN, "invalid_fs_name", 0), ENODEV);
	TEST_ERRNO(syscall(SYS_FSOPEN, "", 0), ENODEV);
}
END_TEST()

FN_TEST(fsopen_null_fsname)
{
	TEST_ERRNO(syscall(SYS_FSOPEN, NULL, 0), EINVAL);
}
END_TEST()

FN_TEST(fsopen_tmpfs)
{
	int fd = TEST_SUCC(syscall(SYS_FSOPEN, "tmpfs", 0));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(fsopen_cloexec)
{
	int fd = TEST_SUCC(syscall(SYS_FSOPEN, "tmpfs", FSOPEN_CLOEXEC));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(fsconfig_and_fsmount)
{
	int fs_fd = TEST_SUCC(syscall(SYS_FSOPEN, "tmpfs", 0));
	TEST_SUCC(syscall(SYS_FSCONFIG, fs_fd, FSCONFIG_SET_STRING, "source",
			  "test_source", 0));
	TEST_SUCC(syscall(SYS_FSCONFIG, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL,
			  0));
	int mount_fd =
		TEST_SUCC(syscall(SYS_FSMOUNT, fs_fd, FSMOUNT_CLOEXEC, 0));
	TEST_SUCC(close(mount_fd));
	TEST_SUCC(close(fs_fd));
}
END_TEST()

FN_TEST(move_mount_flags_zero_nonempty_from_path)
{
	const char *src = "/tmp/move_mount_flags_zero_src";
	const char *dst = "/tmp/move_mount_flags_zero_dst";

	CHECK(unshare(CLONE_NEWNS));
	ensure_dir(src);
	ensure_dir(dst);
	CHECK(mount("tmpfs", src, "tmpfs", 0, NULL));

	TEST_SUCC(syscall(SYS_MOVE_MOUNT, AT_FDCWD, src, AT_FDCWD, dst, 0));

	TEST_SUCC(umount(dst));
	TEST_SUCC(rmdir(src));
	TEST_SUCC(rmdir(dst));
}
END_TEST()

FN_TEST(move_mount_flags_zero_empty_from_path)
{
	const char *dst = "/tmp/move_mount_empty_without_flag_dst";

	CHECK(unshare(CLONE_NEWNS));
	ensure_dir(dst);
	int mount_fd = create_detached_tmpfs();

	TEST_ERRNO(syscall(SYS_MOVE_MOUNT, mount_fd, "", AT_FDCWD, dst, 0),
		   ENOENT);

	TEST_SUCC(close(mount_fd));
	TEST_SUCC(rmdir(dst));
}
END_TEST()

FN_TEST(move_mount_empty_path_detached_mount)
{
	const char *dst = "/tmp/move_mount_empty_detached_dst";

	CHECK(unshare(CLONE_NEWNS));
	ensure_dir(dst);
	int mount_fd = create_detached_tmpfs();

	TEST_SUCC(syscall(SYS_MOVE_MOUNT, mount_fd, "", AT_FDCWD, dst,
			  MOVE_MOUNT_F_EMPTY_PATH));

	TEST_SUCC(close(mount_fd));
	TEST_SUCC(umount(dst));
	TEST_SUCC(rmdir(dst));
}
END_TEST()

FN_TEST(move_mount_empty_path_nonempty_from_path)
{
	const char *src = "/tmp/move_mount_empty_flag_src";
	const char *dst = "/tmp/move_mount_empty_flag_dst";

	CHECK(unshare(CLONE_NEWNS));
	ensure_dir(src);
	ensure_dir(dst);
	CHECK(mount("tmpfs", src, "tmpfs", 0, NULL));

	TEST_SUCC(syscall(SYS_MOVE_MOUNT, AT_FDCWD, src, AT_FDCWD, dst,
			  MOVE_MOUNT_F_EMPTY_PATH));

	TEST_SUCC(umount(dst));
	TEST_SUCC(rmdir(src));
	TEST_SUCC(rmdir(dst));
}
END_TEST()
