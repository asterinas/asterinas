/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <fcntl.h>
#include <limits.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/ext2/open_dir_test"
#define TARGET_DIR TEST_DIR "/objd"

FN_SETUP(prepare)
{
	CHECK(mkdir(TEST_DIR, 0755));
	CHECK(mkdir(TARGET_DIR, 0755));
}
END_SETUP()

FN_TEST(open_creat_directory_on_existing_dir_returns_einval)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_DIRECTORY | O_RDONLY, 0644),
		   EINVAL);
}
END_TEST()

// A regression test for the bug described in #4274.
FN_TEST(open_creat_directory_precedes_pathname_errors)
{
	char long_path[PATH_MAX + 1];
	const char *paths[] = { NULL, "", long_path };
	const int path_errors[] = { EFAULT, ENOENT, ENAMETOOLONG };
	const int flags = O_CREAT | O_DIRECTORY;

	memset(long_path, 'x', PATH_MAX);
	long_path[PATH_MAX] = '\0';
	for (size_t i = 0; i < sizeof(paths) / sizeof(paths[0]); ++i) {
#ifdef SYS_open
		TEST(syscall(SYS_open, paths[i], flags, 0644), EINVAL,
		     _ret == -1);
		TEST(syscall(SYS_open, paths[i], O_PATH | flags, 0644),
		     path_errors[i], _ret == -1);
#endif
		TEST(syscall(SYS_openat, AT_FDCWD, paths[i], flags, 0644),
		     EINVAL, _ret == -1);
		TEST(syscall(SYS_openat, AT_FDCWD, paths[i], O_PATH | flags,
			     0644),
		     path_errors[i], _ret == -1);
	}
}
END_TEST()

FN_TEST(open_creat_excl_directory_on_existing_dir_returns_einval)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_DIRECTORY | O_WRONLY,
			0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_excl_directory_rdwr_on_existing_dir_returns_einval)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_DIRECTORY | O_RDWR,
			0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_read_only_on_existing_dir_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_RDONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_on_existing_dir_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_WRONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_rdwr_on_existing_dir_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_RDWR, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_excl_read_only_on_existing_dir_returns_eexist)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_RDONLY, 0644), EEXIST);
}
END_TEST()

FN_TEST(open_creat_excl_on_existing_dir_returns_eexist)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_WRONLY, 0644), EEXIST);
}
END_TEST()

FN_TEST(open_creat_excl_rdwr_on_existing_dir_returns_eexist)
{
	TEST_ERRNO(open(TARGET_DIR, O_CREAT | O_EXCL | O_RDWR, 0644), EEXIST);
}
END_TEST()

FN_TEST(open_write_only_on_dir_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_DIR, O_WRONLY), EISDIR);
}
END_TEST()

FN_TEST(open_rdwr_on_dir_returns_eisdir)
{
	TEST_ERRNO(open(TARGET_DIR, O_RDWR), EISDIR);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(rmdir(TARGET_DIR));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
