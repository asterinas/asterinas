// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define DIRECTORY "/ext4/extent_remount"
#define FILE_PATH DIRECTORY "/data"
#define BLOCK_SIZE 4096
#define EXTENTS 400
#define KEPT_EXTENTS 371
#define FINAL_BLOCKS (EXTENTS * 3 + 2)

/* The shell runs these phases on opposite sides of a real device remount. */
FN_TEST(sparse_split_truncate_and_remount)
{
	const char *phase = getenv("EXTENT_REMOUNT_PHASE");
	CHECK_WITH(phase != NULL, _ret);
	char buffer[BLOCK_SIZE];
	if (!strcmp(phase, "prepare")) {
		TEST_SUCC(mkdir(DIRECTORY, 0755));
		int fd = TEST_SUCC(
			open(DIRECTORY "/original", O_CREAT | O_RDWR, 0644));
		for (int i = 0; i < EXTENTS; i++) {
			memset(buffer, i % 251 + 1, sizeof(buffer));
			TEST_RES(pwrite(fd, buffer, sizeof(buffer),
					(off_t)i * 3 * BLOCK_SIZE),
				 _ret == sizeof(buffer));
		}
		TEST_SUCC(ftruncate(fd, (KEPT_EXTENTS * 3 - 2) * BLOCK_SIZE));
		TEST_SUCC(ftruncate(fd, FINAL_BLOCKS * BLOCK_SIZE));
		TEST_SUCC(fsync(fd));
		TEST_SUCC(close(fd));
		TEST_SUCC(rename(DIRECTORY "/original", FILE_PATH));
		fd = TEST_SUCC(
			open(DIRECTORY "/unlinked", O_CREAT | O_RDWR, 0644));
		TEST_SUCC(unlink(DIRECTORY "/unlinked"));
		TEST_RES(write(fd, "alive", 5), _ret == 5);
		TEST_RES(pread(fd, buffer, 5, 0), _ret == 5);
		TEST_RES(memcmp(buffer, "alive", 5), _ret == 0);
		TEST_SUCC(close(fd));
	} else {
		CHECK_WITH(strcmp(phase, "verify"), _ret == 0);
		int fd = TEST_SUCC(open(FILE_PATH, O_RDONLY));
		struct stat st;
		TEST_SUCC(fstat(fd, &st));
		TEST_RES(st.st_size, _ret == FINAL_BLOCKS * BLOCK_SIZE);
		for (int block = 0; block < FINAL_BLOCKS; block++) {
			memset(buffer, 0xff, sizeof(buffer));
			TEST_RES(pread(fd, buffer, sizeof(buffer),
				       (off_t)block * BLOCK_SIZE),
				 _ret == sizeof(buffer));
			unsigned char expected =
				block % 3 == 0 && block / 3 < KEPT_EXTENTS ?
					block / 3 % 251 + 1 :
					0;
			for (int byte = 0; byte < BLOCK_SIZE; byte++) {
				if ((unsigned char)buffer[byte] != expected) {
					TEST_RES((unsigned char)buffer[byte],
						 _ret == expected);
					break;
				}
			}
		}
		TEST_SUCC(close(fd));
		TEST_SUCC(unlink(FILE_PATH));
		TEST_SUCC(rmdir(DIRECTORY));
	}
}
END_TEST()
