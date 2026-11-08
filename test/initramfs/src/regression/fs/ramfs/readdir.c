// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp/ramfs_readdir_test"
#define TEST_FILE TEST_DIR "/file"

FN_TEST(seekdir_resumes_after_entry)
{
	const char *entries[] = { ".", "..", "file" };
	const size_t nr_entries = sizeof(entries) / sizeof(entries[0]);
	long offsets[nr_entries];

	TEST_SUCC(mkdir(TEST_DIR, 0755));
	int fd = TEST_SUCC(creat(TEST_FILE, 0644));
	TEST_SUCC(close(fd));

	DIR *dir = TEST_SUCC(opendir(TEST_DIR));
	for (size_t i = 0; i < nr_entries; ++i) {
		TEST_RES(readdir(dir),
			 _ret != NULL && strcmp(_ret->d_name, entries[i]) == 0);
		offsets[i] = TEST_RES(telldir(dir), _ret >= 0);
	}

	for (size_t i = 0; i < nr_entries; ++i) {
		seekdir(dir, offsets[i]);
		TEST_RES(readdir(dir),
			 i + 1 == nr_entries ?
				 _ret == NULL :
				 _ret != NULL && strcmp(_ret->d_name,
							entries[i + 1]) == 0);
	}

	TEST_SUCC(closedir(dir));
	TEST_SUCC(unlink(TEST_FILE));
	TEST_SUCC(rmdir(TEST_DIR));
}
END_TEST()
