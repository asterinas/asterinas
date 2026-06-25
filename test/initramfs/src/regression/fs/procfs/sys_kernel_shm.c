// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

struct shm_sysctl_case {
	const char *name;
	const char *path;
	const char *expected;
};

static const struct shm_sysctl_case SHM_SYSCTL_CASES[] = {
	{ "shmall", "/proc/sys/kernel/shmall", "18446744073692774399\n" },
	{ "shmmax", "/proc/sys/kernel/shmmax", "18446744073692774399\n" },
	{ "shmmni", "/proc/sys/kernel/shmmni", "4096\n" },
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
	for (struct dirent *entry = readdir(dir); entry != NULL;
	     entry = readdir(dir)) {
		if (strcmp(entry->d_name, name) == 0 &&
		    entry->d_type == DT_REG) {
			found = 1;
			break;
		}
	}
	CHECK(closedir(dir));

	return found;
}

FN_TEST(shm_sysctl_files_have_linux_default_values)
{
	for (size_t i = 0;
	     i < sizeof(SHM_SYSCTL_CASES) / sizeof(SHM_SYSCTL_CASES[0]); i++) {
		char buf[64];

		read_file(SHM_SYSCTL_CASES[i].path, buf, sizeof(buf));
		TEST_RES(strcmp(buf, SHM_SYSCTL_CASES[i].expected) == 0,
			 _ret == 0);
	}
}
END_TEST()

FN_TEST(shm_sysctl_files_are_user_writable_regular_files)
{
	for (size_t i = 0;
	     i < sizeof(SHM_SYSCTL_CASES) / sizeof(SHM_SYSCTL_CASES[0]); i++) {
		struct stat st;

		TEST_SUCC(stat(SHM_SYSCTL_CASES[i].path, &st));
		TEST_RES(S_ISREG(st.st_mode), _ret != 0);
		TEST_RES((st.st_mode & 0777) == 0644, _ret == 1);
	}
}
END_TEST()

FN_TEST(shm_sysctl_files_are_listed)
{
	for (size_t i = 0;
	     i < sizeof(SHM_SYSCTL_CASES) / sizeof(SHM_SYSCTL_CASES[0]); i++) {
		TEST_RES(has_dir_entry(SHM_SYSCTL_CASES[i].name), _ret == 1);
	}
}
END_TEST()
