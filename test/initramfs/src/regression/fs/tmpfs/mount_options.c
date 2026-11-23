// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sys/fsuid.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define MOUNT_DIR "/tmp/tmpfs_mount_options"
#define ROOT_FILE MOUNT_DIR "/root_file"
#define USER_FILE MOUNT_DIR "/user_file"

#define USER_UID 1000
#define USER_GID 2000

// Regression tests for the bug described in #4280.

FN_SETUP(create_mount_dir)
{
	CHECK_WITH(mkdir(MOUNT_DIR, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

FN_TEST(default_root_dir)
{
	struct stat st;

	TEST_SUCC(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, NULL));
	TEST_RES(stat(MOUNT_DIR, &st), st.st_mode == (S_IFDIR | 01777) &&
					       st.st_uid == 0 &&
					       st.st_gid == 0);
	TEST_SUCC(umount(MOUNT_DIR));
}
END_TEST()

FN_TEST(default_root_dir_owner_is_fs_ids)
{
	struct stat st;
	int status;
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(setfsgid(USER_GID), _ret == 0);
		CHECK_WITH(setfsuid(USER_UID), _ret == 0);
		CHECK(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, NULL));
		_exit(EXIT_SUCCESS);
	}
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_RES(stat(MOUNT_DIR, &st), st.st_mode == (S_IFDIR | 01777) &&
					       st.st_uid == USER_UID &&
					       st.st_gid == USER_GID);
	TEST_SUCC(umount(MOUNT_DIR));
}
END_TEST()

FN_TEST(explicit_root_dir_mode_and_owner)
{
	char options[64];
	struct stat st;

	CHECK_WITH(snprintf(options, sizeof(options), "mode=0700,uid=%d,gid=%d",
			    USER_UID, USER_GID),
		   _ret > 0 && _ret < sizeof(options));
	TEST_SUCC(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, options));
	TEST_RES(stat(MOUNT_DIR, &st), st.st_mode == (S_IFDIR | 0700) &&
					       st.st_uid == USER_UID &&
					       st.st_gid == USER_GID);
	TEST_SUCC(umount(MOUNT_DIR));

	// `mode` is always octal, while `uid` and `gid` take a `0x` or `0`
	// prefix for hexadecimal or octal.
	CHECK_WITH(snprintf(options, sizeof(options),
			    "size=1m,mode=750,uid=%#x,gid=%#o", USER_UID,
			    USER_GID),
		   _ret > 0 && _ret < sizeof(options));
	TEST_SUCC(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, options));
	TEST_RES(stat(MOUNT_DIR, &st), st.st_mode == (S_IFDIR | 0750) &&
					       st.st_uid == USER_UID &&
					       st.st_gid == USER_GID);
	TEST_SUCC(umount(MOUNT_DIR));
}
END_TEST()

FN_TEST(invalid_option_values)
{
	struct stat dir_st, st;

	TEST_SUCC(stat(MOUNT_DIR, &dir_st));

	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "mode=abc"), EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "mode=0800"), EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "mode="), EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "mode"), EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "uid=abc"), EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "uid=4294967295"),
		   EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "uid=4294967296"),
		   EINVAL);
	TEST_ERRNO(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, "gid=-1"), EINVAL);

	TEST_RES(stat(MOUNT_DIR, &st),
		 st.st_dev == dir_st.st_dev && st.st_ino == dir_st.st_ino);
}
END_TEST()

FN_TEST(non_root_create_in_default_root_dir)
{
	struct stat st;
	int status;
	pid_t pid;
	int fd;

	TEST_SUCC(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, NULL));
	fd = TEST_SUCC(open(ROOT_FILE, O_CREAT | O_EXCL | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setresgid(USER_GID, USER_GID, USER_GID));
		CHECK(setresuid(USER_UID, USER_UID, USER_UID));

		fd = CHECK(open(USER_FILE, O_CREAT | O_EXCL | O_WRONLY, 0644));
		CHECK_WITH(write(fd, "data", 4), _ret == 4);
		CHECK(close(fd));

		// The sticky bit protects entries owned by other users.
		CHECK_WITH(unlink(ROOT_FILE), _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_RES(stat(USER_FILE, &st), st.st_uid == USER_UID &&
					       st.st_gid == USER_GID &&
					       st.st_size == 4);
	TEST_SUCC(stat(ROOT_FILE, &st));

	TEST_SUCC(unlink(USER_FILE));
	TEST_SUCC(unlink(ROOT_FILE));
	TEST_SUCC(umount(MOUNT_DIR));
}
END_TEST()

FN_SETUP(remove_mount_dir)
{
	CHECK(rmdir(MOUNT_DIR));
}
END_SETUP()
