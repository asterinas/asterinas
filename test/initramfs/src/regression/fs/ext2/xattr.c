// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/xattr_test"

static void ensure_base_dir(void)
{
	CHECK_WITH(mkdir(BASE_DIR, 0755), _ret >= 0 || errno == EEXIST);
}

FN_SETUP(prepare_base_dir)
{
	ensure_base_dir();
}
END_SETUP()

FN_TEST(xattr_set_get_roundtrip)
{
	const char *path = BASE_DIR "/roundtrip";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(setxattr(path, "user.test", "value123", 8, 0));

	char buf[64] = { 0 };
	TEST_RES(getxattr(path, "user.test", buf, sizeof(buf)), _ret == 8);
	TEST_RES(strcmp(buf, "value123"), _ret == 0);

	TEST_SUCC(unlink(path));
}
END_TEST()

// TODO: xattr_list test is disabled because sys_listxattr only lists one
// namespace (Trusted when running as root), so user.* xattrs are invisible.
// This is a VFS-level bug, not ext2-specific.

FN_TEST(xattr_remove_enodata)
{
	const char *path = BASE_DIR "/removefile";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(setxattr(path, "user.gone", "tmp", 3, 0));
	TEST_SUCC(removexattr(path, "user.gone"));

	char buf[64] = { 0 };
	TEST_ERRNO(getxattr(path, "user.gone", buf, sizeof(buf)), ENODATA);

	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(xattr_nonexistent_enodata)
{
	const char *path = BASE_DIR "/noattr";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	char buf[64] = { 0 };
	TEST_ERRNO(getxattr(path, "user.nonexistent", buf, sizeof(buf)),
		   ENODATA);

	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(xattr_with_o_path_fd)
{
	const char *path = BASE_DIR "/opath";
	const char name[] = "user.test";
	int val = 1234;
	char buf[sizeof(val)];
	char list[sizeof(name)];

	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	int opath_fd = TEST_SUCC(open(path, O_PATH));
	TEST_ERRNO(syscall(SYS_fsetxattr, opath_fd, name, &val, sizeof(val), 0),
		   EBADF);
	TEST_ERRNO(syscall(SYS_fgetxattr, opath_fd, name, buf, sizeof(buf)),
		   EBADF);
	TEST_ERRNO(syscall(SYS_flistxattr, opath_fd, list, sizeof(list)),
		   EBADF);
	TEST_ERRNO(syscall(SYS_fremovexattr, opath_fd, name), EBADF);
	TEST_SUCC(close(opath_fd));

	TEST_SUCC(unlink(path));
}
END_TEST()
