// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

#define FDATASYNC_FILE "test_fdatasync.txt"
#define FSYNC_FILE "test_fsync.txt"

FN_SETUP(select_filesystem)
{
	CHECK_WITH(getenv("SYNC_TEST_ROOT"), _ret != NULL);
	CHECK(chdir(getenv("SYNC_TEST_ROOT")));
}
END_SETUP()

#define TEST_SYNC_FILE(path, sync_fn)                                          \
	do {                                                                   \
		const char data[] = "Hello, sync test!\n";                     \
		int fd = TEST_SUCC(                                            \
			open(path, O_RDWR | O_CREAT | O_EXCL, 0600));          \
		TEST_RES(write(fd, data, sizeof(data)), _ret == sizeof(data)); \
		TEST_SUCC(sync_fn(fd));                                        \
		char buffer[sizeof(data)];                                     \
		TEST_RES(pread(fd, buffer, sizeof(buffer), 0),                 \
			 _ret == sizeof(buffer) &&                             \
				 memcmp(buffer, data, sizeof(data)) == 0);     \
		TEST_SUCC(close(fd));                                          \
		TEST_SUCC(unlink(path));                                       \
	} while (0)

FN_TEST(fdatasync_file)
{
	TEST_SYNC_FILE(FDATASYNC_FILE, fdatasync);
}
END_TEST()

FN_TEST(fsync_file)
{
	TEST_SYNC_FILE(FSYNC_FILE, fsync);
}
END_TEST()
