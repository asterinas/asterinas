// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

struct uts_sysctl_case {
	const char *path;
	const char *expected;
};

static const struct uts_sysctl_case UTS_SYSCTL_CASES[] = {
	{ "/proc/sys/kernel/ostype", "Linux\n" },
	{ "/proc/sys/kernel/osrelease", "5.13.0\n" },
	{ "/proc/sys/kernel/version", "#1 SMP " },
};

static void read_file(const char *path, char *buf, size_t buf_size)
{
	int fd = CHECK(open(path, O_RDONLY));
	ssize_t bytes = CHECK(read(fd, buf, buf_size - 1));

	buf[bytes] = '\0';
	CHECK(close(fd));
}

static int has_dir_entry(const char *name)
{
	DIR *dir = opendir("/proc/sys/kernel");
	CHECK(dir == NULL ? -1 : 0);

	int found = 0;
	errno = 0;
	for (struct dirent *entry = readdir(dir); entry != NULL;
	     entry = readdir(dir)) {
		if (strcmp(entry->d_name, name) == 0 &&
		    entry->d_type == DT_REG) {
			found = 1;
			break;
		}
		errno = 0;
	}
	CHECK(errno == 0 ? 0 : -1);
	CHECK(closedir(dir));

	return found;
}

FN_TEST(uts_sysctl_files_match_uname_fields)
{
	for (size_t i = 0;
	     i < sizeof(UTS_SYSCTL_CASES) / sizeof(UTS_SYSCTL_CASES[0]); i++) {
		char buf[128];

		read_file(UTS_SYSCTL_CASES[i].path, buf, sizeof(buf));

		if (strcmp(UTS_SYSCTL_CASES[i].path,
			   "/proc/sys/kernel/version") == 0) {
			TEST_RES(
				strncmp(buf, UTS_SYSCTL_CASES[i].expected,
					strlen(UTS_SYSCTL_CASES[i].expected)) ==
					0,
				_ret == 0);
			TEST_RES(strchr(buf, '\n') != NULL, _ret == 1);
		} else {
			TEST_RES(strcmp(buf, UTS_SYSCTL_CASES[i].expected) == 0,
				 _ret == 0);
		}
	}
}
END_TEST()

FN_TEST(uts_sysctl_files_are_read_only_regular_files)
{
	for (size_t i = 0;
	     i < sizeof(UTS_SYSCTL_CASES) / sizeof(UTS_SYSCTL_CASES[0]); i++) {
		struct stat st;

		TEST_SUCC(stat(UTS_SYSCTL_CASES[i].path, &st));
		TEST_RES(S_ISREG(st.st_mode), _ret != 0);
		TEST_RES((st.st_mode & 0777) == 0444, _ret == 1);
		TEST_ERRNO(open(UTS_SYSCTL_CASES[i].path, O_WRONLY), EACCES);
	}
}
END_TEST()

FN_TEST(uts_sysctl_files_are_listed)
{
	TEST_RES(has_dir_entry("ostype"), _ret == 1);
	TEST_RES(has_dir_entry("osrelease"), _ret == 1);
	TEST_RES(has_dir_entry("version"), _ret == 1);
}
END_TEST()
