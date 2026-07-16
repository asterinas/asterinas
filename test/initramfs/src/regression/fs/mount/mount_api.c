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

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret == 0 || errno == EEXIST);
}

static int create_detached_tmpfs(void)
{
	int fs_fd = CHECK(syscall(SYS_fsopen, "tmpfs", 0));
	CHECK(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL, 0));
	int mount_fd = CHECK(syscall(SYS_fsmount, fs_fd, 0, 0));
	CHECK(close(fs_fd));
	return mount_fd;
}

FN_TEST(fsopen_invalid_fsname)
{
	TEST_ERRNO(syscall(SYS_fsopen, "invalid_fs_name", 0), ENODEV);
	TEST_ERRNO(syscall(SYS_fsopen, "", 0), ENODEV);
}
END_TEST()

FN_TEST(fsopen_null_fsname)
{
	TEST_ERRNO(syscall(SYS_fsopen, NULL, 0), EFAULT);
}
END_TEST()

FN_TEST(fsopen_tmpfs)
{
	int fd = TEST_SUCC(syscall(SYS_fsopen, "tmpfs", 0));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(fsopen_cloexec)
{
	int fd = TEST_SUCC(syscall(SYS_fsopen, "tmpfs", FSOPEN_CLOEXEC));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(fsconfig_and_fsmount)
{
	int fs_fd = TEST_SUCC(syscall(SYS_fsopen, "tmpfs", 0));
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_STRING, "source",
			  "test_source", 0));
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL,
			  0));
	int mount_fd =
		TEST_SUCC(syscall(SYS_fsmount, fs_fd, FSMOUNT_CLOEXEC, 0));
	TEST_SUCC(close(mount_fd));
	TEST_SUCC(close(fs_fd));
}
END_TEST()

FN_TEST(fsconfig_reconfigures_after_fsmount)
{
	int fs_fd = TEST_SUCC(syscall(SYS_fsopen, "tmpfs", FSOPEN_CLOEXEC));

	TEST_ERRNO(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_RECONFIGURE, NULL,
			   NULL, 0),
		   EBUSY);
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_STRING, "nr_inodes",
			  "1024", 0));
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL,
			  0));
	TEST_ERRNO(syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_STRING, "size",
			   "1048576", 0),
		   EBUSY);

	int mount_fd =
		TEST_SUCC(syscall(SYS_fsmount, fs_fd, FSMOUNT_CLOEXEC, 0));
	TEST_ERRNO(syscall(SYS_fsmount, fs_fd, FSMOUNT_CLOEXEC, 0), EBUSY);

	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_STRING, "size",
			  "1048576", 0));
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_RECONFIGURE, NULL,
			  NULL, 0));
	TEST_SUCC(
		syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_FLAG, "ro", NULL, 0));
	TEST_SUCC(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_RECONFIGURE, NULL,
			  NULL, 0));

	TEST_SUCC(close(mount_fd));
	TEST_SUCC(close(fs_fd));
}
END_TEST()

FN_TEST(fsconfig_rejects_operations_after_creation_failure)
{
	int fs_fd = TEST_SUCC(syscall(SYS_fsopen, "ext2", FSOPEN_CLOEXEC));

	TEST_ERRNO(syscall(SYS_fsconfig, fs_fd, FSCONFIG_CMD_CREATE, NULL, NULL,
			   0),
		   EINVAL);
	TEST_ERRNO(syscall(SYS_fsconfig, fs_fd, FSCONFIG_SET_FLAG, "ro", NULL,
			   0),
		   EBUSY);

	TEST_SUCC(close(fs_fd));
}
END_TEST()

FN_TEST(move_mount_flags_zero_nonempty_from_path)
{
	const char *src = "/tmp/move_mount_flags_zero_src";
	const char *dst = "/tmp/move_mount_flags_zero_dst";

	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir(src);
	ensure_dir(dst);
	TEST_SUCC(mount("tmpfs", src, "tmpfs", 0, NULL));

	TEST_SUCC(syscall(SYS_move_mount, AT_FDCWD, src, AT_FDCWD, dst, 0));

	TEST_SUCC(umount(dst));
	TEST_SUCC(rmdir(src));
	TEST_SUCC(rmdir(dst));
}
END_TEST()

FN_TEST(move_mount_flags_zero_empty_from_path)
{
	const char *dst = "/tmp/move_mount_empty_without_flag_dst";

	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir(dst);
	int mount_fd = create_detached_tmpfs();

	TEST_ERRNO(syscall(SYS_move_mount, mount_fd, "", AT_FDCWD, dst, 0),
		   ENOENT);

	TEST_SUCC(close(mount_fd));
	TEST_SUCC(rmdir(dst));
}
END_TEST()

FN_TEST(move_mount_empty_path_detached_mount)
{
	const char *dst = "/tmp/move_mount_empty_detached_dst";

	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir(dst);
	int mount_fd = create_detached_tmpfs();

	TEST_SUCC(syscall(SYS_move_mount, mount_fd, "", AT_FDCWD, dst,
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

	TEST_SUCC(unshare(CLONE_NEWNS));
	ensure_dir(src);
	ensure_dir(dst);
	TEST_SUCC(mount("tmpfs", src, "tmpfs", 0, NULL));

	TEST_SUCC(syscall(SYS_move_mount, AT_FDCWD, src, AT_FDCWD, dst,
			  MOVE_MOUNT_F_EMPTY_PATH));

	TEST_SUCC(umount(dst));
	TEST_SUCC(rmdir(src));
	TEST_SUCC(rmdir(dst));
}
END_TEST()
